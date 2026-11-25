//! The Loader end to end, outside a zone: file queue, scripted LLM, a fake
//! msg-center. Hosting and recovery, Agent State over real kRPC, the UI
//! tunnel in both directions, replies across a restart, back-pressure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_tool::xllm::XllmDeps;
use async_trait::async_trait;
use buckyos_api::{
    AgentSettings, AiMessage, AiResponse, AiRole, AiUsage, MailboxAddress, MailboxKind,
    MailboxRecord, MailboxRecordWithObject, MsgEditCapability, PostSendResult, RecipientState,
    SessionMemoryMode, TaskWaitReason,
};
use libopendan::api::create_session;
use libopendan::channel::{KmsgChannels, PollWaker};
use libopendan::protocol::*;
use libopendan::runner::RunnerOptions;
use libopendan::state::krpc::{KrpcTransport, StateTransport};
use libopendan::state::{connect, AgentStateClient, ConnectOptions, StateLocator};
use libopendan::{OpenDanError, SessionDir, SessionTemplate};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::{LLMComputeError, ProviderFailure};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, ObjId};
use opendan::loader::{Loader, LoaderEnv, SpecRef};
use opendan::records::{self, AgentRecords, LocalRecords};
use opendan::service::{Access, SERVICE_PATH, WEB_API_PATH};
use opendan::tasks::{NewTask, TaskService, TaskView};
use opendan::ui::MailService;
use serde_json::{json, Value};

#[cfg(unix)]
mod fake_browser;

const AGENT: &str = "did:bns:jarvis.alice";
const WHO: &str = "app:jarvis@alice";
const BOB: &str = "did:bns:bob";

struct Llm {
    calls: AtomicUsize,
    /// Calls that fail before the script answers.
    failures: AtomicUsize,
    /// What the failing calls are: a provider error, or this policy refusal.
    refusal: std::sync::Mutex<Option<&'static str>>,
    script: Box<dyn Fn(&LlmInferenceRequest, usize) -> String + Send + Sync>,
}

impl Llm {
    fn new(f: impl Fn(&LlmInferenceRequest, usize) -> String + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
            refusal: std::sync::Mutex::new(None),
            script: Box::new(f),
        })
    }
}

#[async_trait]
impl LlmClient for Llm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        if self.failures.load(Ordering::SeqCst) > 0 {
            self.failures.fetch_sub(1, Ordering::SeqCst);
            let refusal = *self.refusal.lock().unwrap();
            return Err(LLMComputeError::Provider {
                failure: refusal
                    .map(|code| ProviderFailure::Refused { code: code.to_string() })
                    .unwrap_or(ProviderFailure::Unknown),
                message: "provider refused the request".into(),
            });
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        // `TOOL:<command>` is a shell call instead of an answer.
        let answer = (self.script)(&req, n);
        // `CALL:<function> <json args>` calls any function.
        if let Some(call) = answer.strip_prefix("CALL:") {
            let (name, args) = call.split_once(' ').unwrap();
            let args: serde_json::Map<String, Value> = serde_json::from_str(args).unwrap();
            let mut r = AiResponse::new(AiMessage::new(
                AiRole::Assistant,
                vec![buckyos_api::AiContent::tool_use(format!("c{n}"), name, args.into_iter().collect())],
            ));
            r.usage = Some(AiUsage { input_tokens: Some(10), output_tokens: Some(5), total_tokens: Some(15), ..Default::default() });
            return Ok(r);
        }
        let message = match (answer.strip_prefix("TOOL:"), answer.strip_prefix("REPORT:")) {
            (Some(command), _) => AiMessage::new(
                AiRole::Assistant,
                vec![buckyos_api::AiContent::tool_use(
                    format!("c{n}"),
                    "shell",
                    [("command".to_string(), json!(command))].into_iter().collect(),
                )],
            ),
            // `REPORT:<json args>` is a call of the report tool.
            (_, Some(args)) => {
                let args: serde_json::Map<String, Value> = serde_json::from_str(args).unwrap();
                AiMessage::new(
                    AiRole::Assistant,
                    vec![buckyos_api::AiContent::tool_use(
                        format!("c{n}"),
                        "report",
                        args.into_iter().collect(),
                    )],
                )
            }
            _ => AiMessage::text(AiRole::Assistant, answer),
        };
        let mut r = AiResponse::new(message);
        r.usage = Some(AiUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: Some(15),
            ..Default::default()
        });
        Ok(r)
    }
}

fn cfg_max_tokens(llm_context: &Value) -> Option<u64> {
    llm_context.get("max_tokens").and_then(Value::as_u64)
}

fn last_user(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
        .unwrap_or_default()
}

/// msg-center as far as the tunnel uses it.
#[derive(Default)]
struct Mail {
    records: Mutex<Vec<(MailboxRecord, Option<MsgObject>)>>,
    sent: Mutex<Vec<(String, MsgObject)>>,
    send_calls: AtomicUsize,
    unreachable: AtomicBool,
    authentication_unavailable: AtomicBool,
    /// The conversations can replace a message in place.
    editable: AtomicBool,
    seq: AtomicUsize,
    /// How the owner created a session (by its id).
    memory_modes: Mutex<std::collections::BTreeMap<String, SessionMemoryMode>>,
}

impl Mail {
    /// A message arriving in the agent's inbox `session`.
    fn deliver(&self, session: &str, msg: MsgObject) -> String {
        self.deliver_from(session, msg, "Bob")
    }

    fn deliver_from(&self, session: &str, msg: MsgObject, from_name: &str) -> String {
        let n = self.seq.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        let agent = parse_did(AGENT).unwrap();
        let record = MailboxRecord {
            mailbox: MailboxAddress::new(agent.clone(), Some(session.to_string())).unwrap(),
            record_id: format!("r-{n}"),
            owner: agent.clone(),
            box_kind: MailboxKind::Inbox,
            msg_id: ObjId::new(&msg_key(&msg)).unwrap(),
            msg_kind: msg.kind,
            state: RecipientState::Unread,
            from: msg.from.clone(),
            from_name: Some(from_name.into()),
            to: agent,
            session_id: Some(session.to_string()),
            sort_key: n,
            tags: Vec::new(),
            ingress: None,
            created_at_ms: n,
            updated_at_ms: n,
        };
        let id = record.record_id.clone();
        self.records.lock().unwrap().push((record, Some(msg)));
        id
    }

    fn unread(&self) -> usize {
        self.records
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r.state == RecipientState::Unread)
            .count()
    }

    fn sent(&self) -> Vec<(String, MsgObject)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl MailService for Mail {
    async fn list_inboxes(&self, _owner: &DID) -> Result<Vec<MailboxAddress>, String> {
        if self.authentication_unavailable.load(Ordering::SeqCst) {
            return Err("authentication unavailable: host authorization required".into());
        }
        let mut out: Vec<MailboxAddress> = Vec::new();
        for (r, _) in self.records.lock().unwrap().iter() {
            if !out.contains(&r.mailbox) {
                out.push(r.mailbox.clone());
            }
        }
        Ok(out)
    }

    async fn next_unread(
        &self,
        mailbox: &MailboxAddress,
    ) -> Result<Option<MailboxRecordWithObject>, String> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|(r, _)| &r.mailbox == mailbox && r.state == RecipientState::Unread)
            .map(|(r, m)| MailboxRecordWithObject {
                record: r.clone(),
                msg: m.clone(),
            }))
    }

    async fn mark_read(&self, record_id: &str) -> Result<(), String> {
        for (r, _) in self.records.lock().unwrap().iter_mut() {
            if r.record_id == record_id {
                r.state = RecipientState::Read;
            }
        }
        Ok(())
    }

    async fn recent_read(
        &self,
        mailbox: &MailboxAddress,
        limit: usize,
    ) -> Result<Vec<MailboxRecordWithObject>, String> {
        let read: Vec<MailboxRecordWithObject> = self
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| &r.mailbox == mailbox && r.state == RecipientState::Read)
            .map(|(r, m)| MailboxRecordWithObject {
                record: r.clone(),
                msg: m.clone(),
            })
            .collect();
        Ok(read[read.len().saturating_sub(limit)..].to_vec())
    }

    async fn memory_mode(&self, _owner: &DID, session_id: &str) -> Result<SessionMemoryMode, String> {
        Ok(self.memory_modes.lock().unwrap().get(session_id).copied().unwrap_or_default())
    }

    async fn sender_of(&self, msg_id: &ObjId) -> Result<Option<DID>, String> {
        let sent = self.sent().into_iter().map(|(_, m)| m);
        let received = self.records.lock().unwrap().iter().filter_map(|(_, m)| m.clone()).collect::<Vec<_>>();
        Ok(sent
            .chain(received)
            .find(|m| msg_key(m) == msg_id.to_string())
            .map(|m| m.from))
    }

    async fn post_send(&self, msg: MsgObject, key: &str) -> Result<PostSendResult, String> {
        self.send_calls.fetch_add(1, Ordering::SeqCst);
        if self.unreachable.load(Ordering::SeqCst) {
            return Err("msg-center unreachable".into());
        }
        let mut sent = self.sent.lock().unwrap();
        // The idempotency key returns the first result.
        let first = match sent.iter().find(|(k, _)| k == key) {
            Some((_, m)) => m.clone(),
            None => {
                sent.push((key.to_string(), msg.clone()));
                msg
            }
        };
        Ok(PostSendResult {
            ok: true,
            msg_id: ObjId::new(&msg_key(&first)).unwrap(),
            deliveries: Vec::new(),
            reason: None,
        })
    }

    async fn edit_capability(&self, _msg: MsgObject) -> Result<MsgEditCapability, String> {
        Ok(MsgEditCapability {
            editable: self.editable.load(Ordering::SeqCst),
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone)]
struct FakeTask {
    id: String,
    new: NewTask,
    /// accepted | running | waiting | succeeded | failed | canceled.
    state: String,
    message: String,
    cancel_requested: bool,
}

/// TaskMgr as far as the Loader uses it.
#[derive(Default)]
struct Tasks {
    tasks: Mutex<Vec<FakeTask>>,
    /// Every wait reported, in order.
    waits: Mutex<Vec<(String, TaskWaitReason)>>,
}

impl Tasks {
    fn all(&self) -> Vec<FakeTask> {
        self.tasks.lock().unwrap().clone()
    }

    fn children_of(&self, parent: &str) -> Vec<FakeTask> {
        self.all()
            .into_iter()
            .filter(|t| t.new.parent_id.as_deref() == Some(parent))
            .collect()
    }

    fn set(&self, task_id: &str, f: impl FnOnce(&mut FakeTask)) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap();
        let t = tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("task_not_found: {task_id}"))?;
        if !matches!(t.state.as_str(), "succeeded" | "failed" | "canceled") {
            f(t);
        }
        Ok(())
    }
}

fn task_view(t: &FakeTask) -> TaskView {
    TaskView {
        task_id: t.id.clone(),
        terminal: matches!(t.state.as_str(), "succeeded" | "failed" | "canceled"),
        canceled: t.state == "canceled",
        cancel_requested: t.cancel_requested,
    }
}

#[async_trait]
impl TaskService for Tasks {
    async fn create(&self, new: NewTask) -> Result<TaskView, String> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(t) = tasks.iter().find(|t| t.new.idempotency_key == new.idempotency_key) {
            if t.new.input != new.input {
                return Err("idempotency_conflict".into());
            }
            return Ok(task_view(t));
        }
        let t = FakeTask {
            id: format!("t-{}", tasks.len() + 1),
            new,
            state: "accepted".into(),
            message: String::new(),
            cancel_requested: false,
        };
        tasks.push(t.clone());
        Ok(task_view(&t))
    }

    async fn get(&self, task_id: &str) -> Result<TaskView, String> {
        self.all()
            .iter()
            .find(|t| t.id == task_id)
            .map(task_view)
            .ok_or_else(|| format!("task_not_found: {task_id}"))
    }

    async fn running(&self, task_id: &str, message: &str, _progress: Value) -> Result<(), String> {
        self.set(task_id, |t| {
            t.state = "running".into();
            t.message = message.to_string();
        })
    }

    async fn waiting(&self, task_id: &str, reason: TaskWaitReason) -> Result<(), String> {
        self.waits.lock().unwrap().push((task_id.to_string(), reason.clone()));
        self.set(task_id, |t| {
            t.state = "waiting".into();
            t.message = reason.message.unwrap_or_default();
        })
    }

    async fn complete(&self, task_id: &str, result: Value) -> Result<(), String> {
        self.set(task_id, |t| {
            t.state = "succeeded".into();
            t.message = result["summary"].as_str().unwrap_or_default().to_string();
        })
    }

    async fn fail(&self, task_id: &str, code: &str, _message: &str, _detail: Option<Value>) -> Result<(), String> {
        self.set(task_id, |t| {
            t.state = "failed".into();
            t.message = code.to_string();
        })
    }

    async fn canceled(&self, task_id: &str) -> Result<(), String> {
        self.set(task_id, |t| t.state = "canceled".into())
    }
}

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    queue: PathBuf,
    mail: Arc<Mail>,
    /// `Some`: the Loader has a task service.
    tasks: Option<Arc<Tasks>>,
    /// The agent's settings / profile / info records.
    records: Arc<LocalRecords>,
    settings: AgentSettings,
    /// `Some`: the Loader has a notification service.
    notify: Option<Arc<libopendan::notify::LocalNotify>>,
}

fn doc_id(n: u8) -> ObjId {
    ObjId::new(&format!("agentdoc:{}", format!("{n:02x}").repeat(32))).unwrap()
}

const AGENT_TOML: &str = r#"
[[loader.ui]]
on = "msg.chat"
session_class = "ui"
sid_strategy = "per_peer"

[[loader.ui]]
on = "msg.group"
session_class = "group"
sid_strategy = "per_group"

[session.group]
base = "ui"

# The fake browser (tests/fake_browser) is a headless one: it has no display.
[loader.browser]
display = "headless"
"#;

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("agent_root");
        std::fs::create_dir_all(root.join("i18n")).unwrap();
        std::fs::write(root.join("agent.toml"), AGENT_TOML).unwrap();
        std::fs::write(
            root.join("i18n/en.toml"),
            "[outbound]\ndelivery_failure_notice = \"(the reply could not be delivered)\"\nturn_failed = \"(something went wrong)\"\n",
        )
        .unwrap();
        Self {
            queue: tmp.path().join("kmsg"),
            root,
            _tmp: tmp,
            mail: Arc::new(Mail::default()),
            tasks: None,
            records: Arc::new(LocalRecords::memory()),
            settings: AgentSettings::default(),
            notify: None,
        }
    }

    /// A world with TaskMgr, conversations that can be edited, and an agent
    /// package that sends placeholders after 100 ms.
    fn with_tasks() -> Self {
        let mut w = Self::new();
        std::fs::write(
            w.root.join("agent.toml"),
            format!("[loader]\nplaceholder_delay_ms = 100\n{AGENT_TOML}"),
        )
        .unwrap();
        std::fs::write(
            w.root.join("i18n/en.toml"),
            "[outbound]\nturn_failed = \"(something went wrong)\"\naccepted = \"(working)\"\nstopped = \"(stopped)\"\n",
        )
        .unwrap();
        w.mail.editable.store(true, Ordering::SeqCst);
        w.tasks = Some(Arc::new(Tasks::default()));
        w
    }

    fn channels(&self) -> Arc<KmsgChannels> {
        Arc::new(KmsgChannels::dir(&self.queue).unwrap())
    }

    /// The Loader of the agent bound to AgentSpec generation 1, owned by
    /// Bob; the settings are in the records as the zone keeps them.
    async fn env(&self, llm: Arc<dyn LlmClient>, port: u16) -> LoaderEnv {
        self.records
            .put(records::SETTINGS, serde_json::to_value(&self.settings).unwrap())
            .await
            .unwrap();
        LoaderEnv {
            who: WHO.into(),
            agent_did: AGENT.into(),
            agent_id: None,
            owner_did: Some(parse_did(BOB).unwrap()),
            spec: Some(SpecRef {
                agent_doc_object_id: doc_id(1),
                generation: 1,
            }),
            settings: self.settings.clone(),
            records: self.records.clone(),
            agent_root: self.root.clone(),
            package_root: None,
            skills_package_root: None,
            package_version: None,
            channels: self.channels(),
            waker: Arc::new(PollWaker),
            kevent: None,
            mail: Some(self.mail.clone()),
            tasks: self.tasks.clone().map(|t| t as Arc<dyn TaskService>),
            objects: None,
            notify: self
                .notify
                .clone()
                .map(|n| n as Arc<dyn libopendan::notify::NotifyHost>),
            share: None,
            access: Access::Open { who: WHO.into() },
            port,
            web_dir: None,
            xllm: XllmDeps::default().with_llm(llm),
            session_cli: None,
            session_helpers: Vec::new(),
            context_windows: None,
            options: RunnerOptions {
                poll_interval: Duration::from_millis(50),
                max_wait: Duration::from_millis(300),
                load_hints: false,
                ..Default::default()
            },
        }
    }

    async fn start(&self, llm: Arc<dyn LlmClient>) -> Loader {
        Loader::start(self.env(llm, free_port()).await).await.unwrap()
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ok() {
        assert!(started.elapsed() < Duration::from_secs(20), "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

fn chat(from: &str, text: &str) -> MsgObject {
    text_msg(&parse_did(from).unwrap(), &parse_did(AGENT).unwrap(), text)
}

fn work_spec(objective: &str, first: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.prompt.initial_inputs = vec![PostedInput::text(WHO, AGENT, first).unwrap()];
    spec
}

async fn new_work(agent: &dyn AgentStateClient, w: &World, objective: &str) -> SessionDir {
    create_session(
        &w.root.join("sessions"),
        work_spec(objective, "go"),
        agent,
        WHO,
        w.channels().as_ref(),
    )
    .await
    .unwrap()
}

fn fs_agent(w: &World) -> Arc<dyn AgentStateClient> {
    Arc::new(
        libopendan::FsAgentStateClient::open(&w.root, AGENT, Some(w.channels().client()), None)
            .unwrap(),
    )
}

fn finished(sd: &SessionDir) -> bool {
    sd.state().map(|s| s.is_finished()).unwrap_or(false)
}

/// The registry's copy of a session's status: what the service decides on,
/// published after the session's own state.
fn registered_status(agent: &dyn AgentStateClient, sid: &str) -> Option<SessionStatus> {
    futures_block(agent.sessions().lookup(sid)).ok().flatten().map(|e| e.status)
}

/// M1: sessions registered for this driver are hosted without anything
/// touching them — those left over from before the start and those created
/// while the Loader runs. A work left unfinished by an earlier process is
/// not continued by itself: it is interrupted until its owner resumes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registered_sessions_are_hosted_and_recovered() {
    let w = World::new();
    std::fs::create_dir_all(w.root.join("sessions")).unwrap();
    let before = new_work(fs_agent(&w).as_ref(), &w, "left from an earlier process").await;
    assert_eq!(before.state().unwrap().run_state, RunState::Created);

    // A work session ends with its explicit final report.
    let llm = Llm::new(|_, _| r#"CALL:report {"report": "done", "is_end": true, "outcome": "succeeded"}"#.into());
    let loader = w.start(llm.clone()).await;
    until("the work left unfinished is interrupted", || {
        before.state().is_ok_and(|s| s.paused.as_ref().is_some_and(|p| p.source == PauseSource::Interrupted))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "nothing is inferred before the resume");
    loader
        .agent
        .sessions()
        .post_input(before.sid(), &PostedInput::control(WHO, "resume-1", ControlCommand::Resume { reason: None }))
        .await
        .unwrap();
    until("recovered session finishes", || finished(&before)).await;
    let later = new_work(loader.agent.as_ref(), &w, "created while hosting").await;
    until("new session finishes", || finished(&later)).await;
    assert_eq!(later.state().unwrap().outcome, Some(Outcome::Succeeded));
    // The drive that finished a session returns after its last commit.
    for sd in [&before, &later] {
        until("the finishing drive is counted", || {
            loader
                .supervisor
                .status()
                .iter()
                .any(|h| h.session_id == sd.sid() && h.drives >= 1)
        })
        .await;
    }
    // Sessions of another driver are left alone.
    let mut spec = work_spec("someone else's", "go");
    spec.driver = Some("app:other@alice".into());
    let foreign = create_session(
        &w.root.join("sessions"),
        spec,
        loader.agent.as_ref(),
        WHO,
        w.channels().as_ref(),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(foreign.state().unwrap().run_state, RunState::Created);
    assert!(!loader.supervisor.status().iter().any(|h| h.session_id == foreign.sid()));
    loader.shutdown().await;
}

async fn rpc(port: u16, method: &str, params: Value) -> libopendan::Result<Value> {
    KrpcTransport::new(&format!("http://127.0.0.1:{port}{SERVICE_PATH}"), None)
        .call(method, params)
        .await
}

/// The runtimes an agent was assigned, as its owner switches them on the
/// agent's side (runtime lifecycle TODO R7, R8, RT1): switching the default
/// off keeps it configured; the private runtime stays; removing one keeps its
/// record as history; work in progress on a runtime that goes away is shown
/// first, marked and told to the owner once.
struct NoNode;

#[async_trait]
impl agent_tool::runtime::node::NodeExecutorConnector for NoNode {
    async fn connect(&self, _node: &str) -> std::io::Result<Box<dyn agent_tool::runtime::node::NodeIo>> {
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "no node in this test"))
    }

    async fn session_token(&self) -> std::result::Result<String, String> {
        Ok("test-session".into())
    }
}

struct LabNode;

#[async_trait]
impl agent_tool::runtime::node::NodeExecutorConnector for LabNode {
    async fn connect(&self, _node: &str) -> std::io::Result<Box<dyn agent_tool::runtime::node::NodeIo>> {
        use tokio::io::AsyncWriteExt;
        let (client, mut server) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let mut profile = serde_json::Value::Null;
            while let Ok(Some(buckyos_api::ExecutorFrame::Control(bytes))) = buckyos_api::read_executor_frame(&mut server).await {
                let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let id = &request["request_id"];
                let response = match request["type"].as_str().unwrap() {
                    "login" => {
                        profile = request["profile"].clone();
                        json!({"type":"login_ok", "request_id":id, "posture":"normal", "principal":AGENT, "app_instance_id":"jarvis@alice",
                            "caps":["open_stream","describe","exec","read","write","mkdir","stat","query","cancel"],
                            "limits":buckyos_api::ExecutorLimits::default(), "revision":"one", "generation":1,
                            "auth_valid_until_ms":u64::MAX, "token_expires_at_ms":u64::MAX, "store_id":"lab-ledger", "server_epoch":"one"})
                    }
                    "describe" => json!({"type":"describe_ok", "request_id":id,
                        "target":{"node_did":"did:web:lab.test.buckyos.io", "profile_id":profile, "uid":1000, "gid":1000,
                            "workdir":request["workdir"], "workdir_roots":["/lab"], "store_id":"lab-ledger", "protocol_version":1},
                        "info":{"os":"linux", "arch":"x86_64", "hostname":"lab", "shell":"bash", "time_ms":1, "home":"/lab/home", "identity_mode":"current_user"}}),
                    other => json!({"type":"error", "request_id":id, "error":{"code":"capability", "message":format!("lab node: no {other}")}}),
                };
                server.write_all(&buckyos_api::encode_executor_control(&response).unwrap()).await.unwrap();
            }
        });
        Ok(Box::new(client))
    }

    async fn session_token(&self) -> std::result::Result<String, String> {
        Ok("test-session".into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_switches_assigned_runtimes_and_work_in_progress_is_told() {
    use buckyos_api::{AgentRuntimeSettings, AgentRuntimeTarget};
    libopendan::runtime::set_node_connector(Arc::new(LabNode));
    let mut w = World::new();
    w.settings.runtimes.insert(
        "node-lab".into(),
        AgentRuntimeSettings {
            enabled: true,
            target: AgentRuntimeTarget::Node { node_id: "lab".into(), node_did: "did:web:lab.test.buckyos.io".into() },
        },
    );
    w.settings.default_runtime_id = Some("node-lab".into());
    let notify = Arc::new(libopendan::notify::LocalNotify::enable(&w.root, AGENT).unwrap());
    w.notify = Some(notify.clone());
    let port = free_port();
    let llm = Llm::new(|req, _| format!("echo: {}", last_user(req).len()));
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;

    let settings = rpc(port, "runtimes.settings", json!({})).await.unwrap();
    assert_eq!((settings["default_runtime_id"].as_str(), settings["configured_default_runtime_id"].as_str()), (Some("node-lab"), Some("node-lab")));
    assert_eq!(settings["runtimes"]["node-lab"], json!({"enabled": true, "target": {"kind": "node", "node_id": "lab", "node_did": "did:web:lab.test.buckyos.io"}}));

    // A work that is not finished, frozen to the lab runtime.
    let done = new_work(loader.agent.as_ref(), &w, "a finished one").await;
    until("work session finishes", || finished(&done)).await;
    let mut entry = loader.agent.sessions().lookup(done.sid()).await.unwrap().unwrap();
    assert_eq!(entry.runtime_id.as_deref(), Some("private"));
    entry.session_id = "work-20260101-900".into();
    entry.idempotency_key = Some("on-the-lab".into());
    entry.driver.principal = "app:elsewhere@alice".into();
    entry.runtime_id = Some("node-lab".into());
    entry.title = Some("render on the lab".into());
    entry.status = SessionStatus::created(libopendan::now_ms());
    let mut idle = entry.clone();
    idle.session_id = "work-20260101-899".into();
    idle.idempotency_key = Some("idle-on-the-lab".into());
    loader.agent.sessions().register(idle, WHO).await.unwrap();

    // Created but never started: switching the runtime off cuts nothing off.
    let impact = rpc(port, "runtimes.impact", json!({"runtime_id": "node-lab"})).await.unwrap();
    assert_eq!((impact["blocking"].as_bool(), impact["active_sessions"].as_array().map(Vec::len)), (Some(false), Some(1)));
    entry.status.run_state = RunState::Running;
    loader.agent.sessions().register(entry, WHO).await.unwrap();

    let impact = rpc(port, "runtimes.impact", json!({"runtime_id": "node-lab"})).await.unwrap();
    assert_eq!((impact["blocking"].as_bool(), impact["owner_default"].as_bool()), (Some(true), Some(true)));
    assert_eq!(impact["running_sessions"], json!(["work-20260101-900"]));
    assert_eq!(impact["active_sessions"].as_array().map(Vec::len), Some(2));
    assert_eq!(rpc(port, "runtimes.impact", json!({"runtime_id": "private"})).await.unwrap()["active_sessions"], json!([]));

    // A runtime is not taken away from sessions in progress on it.
    for (method, params) in [
        ("runtimes.settings_set", json!({"runtime_id": "node-lab", "enabled": false})),
        ("runtimes.settings_set", json!({"runtime_id": "node-lab", "enabled": false, "confirm": true})),
        ("runtimes.remove", json!({"runtime_id": "node-lab"})),
    ] {
        let refused = rpc(port, method, params).await.unwrap_err().to_string();
        assert!(refused.contains("sessions_in_progress") && refused.contains("work-20260101-900"), "{refused}");
    }
    assert_eq!(rpc(port, "runtimes.settings", json!({})).await.unwrap()["runtimes"]["node-lab"]["enabled"], true);

    // Switched off past the agent (the control panel's `agent.update`): a
    // default that is off keeps its place.
    let mut changed = records::settings(w.records.as_ref()).await.unwrap();
    changed.runtimes.get_mut("node-lab").unwrap().enabled = false;
    records::set_settings(w.records.as_ref(), &changed).await.unwrap();
    opendan::loader::project_runtimes(loader.agent.as_ref(), loader.agent.agent_root().unwrap(), &changed, WHO).await;
    let off = rpc(port, "runtimes.settings", json!({})).await.unwrap();
    assert_eq!((off["default_runtime_id"].as_str(), off["configured_default_runtime_id"].as_str()), (Some("private"), Some("node-lab")));
    let record = loader.agent.runtimes().lookup("node-lab").await.unwrap().unwrap();
    assert!(!record.usable() && record.lifecycle == RuntimeLifecycle::Active);

    // The session stays where it was frozen: marked, and the owner is told once.
    opendan::loader::mark_runtime_sessions(loader.agent.as_ref(), Some(notify.as_ref())).await;
    opendan::loader::mark_runtime_sessions(loader.agent.as_ref(), Some(notify.as_ref())).await;
    let marked = loader.agent.sessions().lookup("work-20260101-900").await.unwrap().unwrap();
    assert!(marked.runtime_unavailable.as_deref().unwrap().contains("switched off"), "{:?}", marked.runtime_unavailable);
    assert_eq!(marked.runtime_id.as_deref(), Some("node-lab"), "never moved");
    use libopendan::notify::NotifyHost;
    let told = notify.list(Default::default()).await.unwrap();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].prompt.message.as_deref().unwrap().contains("work-20260101-900"), "{:?}", told[0].prompt);

    // Back on: the default counts again and the mark goes.
    let on = rpc(port, "runtimes.settings_set", json!({"runtime_id": "node-lab", "enabled": true})).await.unwrap();
    assert_eq!(on["default_runtime_id"], "node-lab");
    opendan::loader::mark_runtime_sessions(loader.agent.as_ref(), Some(notify.as_ref())).await;
    assert!(loader.agent.sessions().lookup("work-20260101-900").await.unwrap().unwrap().runtime_unavailable.is_none());

    // The private runtime is always on and cannot be removed, under any of its names.
    for id in ["private", "container"] {
        assert!(rpc(port, "runtimes.settings_set", json!({"runtime_id": id, "enabled": false})).await.unwrap_err().to_string().contains("always on"));
        assert!(rpc(port, "runtimes.remove", json!({"runtime_id": id})).await.unwrap_err().to_string().contains("cannot be removed"));
    }
    assert!(rpc(port, "runtimes.settings_set", json!({"runtime_id": "node-other", "enabled": true})).await.unwrap_err().to_string().contains("not assigned"));

    // Removed past the agent: gone from its settings with the default, kept as history.
    let mut changed = records::settings(w.records.as_ref()).await.unwrap();
    changed.runtimes.remove("node-lab");
    changed.default_runtime_id = None;
    records::set_settings(w.records.as_ref(), &changed).await.unwrap();
    opendan::loader::project_runtimes(loader.agent.as_ref(), loader.agent.agent_root().unwrap(), &changed, WHO).await;
    let removed = rpc(port, "runtimes.settings", json!({})).await.unwrap();
    assert_eq!(removed["runtimes"], json!({}));
    assert_eq!((removed["default_runtime_id"].as_str(), removed["configured_default_runtime_id"].as_str()), (Some("private"), None));
    let retired = loader.agent.runtimes().lookup("node-lab").await.unwrap().unwrap();
    assert_eq!(retired.lifecycle, RuntimeLifecycle::Retired);
    opendan::loader::mark_runtime_sessions(loader.agent.as_ref(), Some(notify.as_ref())).await;
    let marked = loader.agent.sessions().lookup("work-20260101-900").await.unwrap().unwrap();
    assert!(marked.runtime_unavailable.as_deref().unwrap().contains("removed from this agent"));
    let mut spec = work_spec("on the lab again", "go");
    spec.runtime.requirement.runtime_id = Some("node-lab".into());
    let refused = create_session(&w.root.join("sessions"), spec, loader.agent.as_ref(), WHO, w.channels().as_ref()).await.unwrap_err();
    assert!(refused.to_string().contains("retired"), "{refused}");
    loader.shutdown().await;
}

/// The owner pauses, resumes and stops a plain work session: its queue
/// takes these controls (and no message); the stop interrupts the command
/// in flight and the work ends as stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_controls_a_work_session_that_takes_no_message() {
    let w = World::new();
    let asked = Arc::new(AtomicBool::new(false));
    let seen = asked.clone();
    let llm = Llm::new(move |_, _| {
        seen.store(true, Ordering::SeqCst);
        "TOOL:sleep 20".to_string()
    });
    let port = free_port();
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    let work = new_work(loader.agent.as_ref(), &w, "a long job").await;
    assert!(loader.agent.sessions().lookup(work.sid()).await.unwrap().unwrap().input_queue.is_some());
    until("the work runs its command", || asked.load(Ordering::SeqCst)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let refused = rpc(port, "session.post", json!({ "sid": work.sid(), "text": "do it differently" })).await.unwrap_err();
    assert!(refused.to_string().contains("takes no messages"), "{refused}");
    let paused = rpc(port, "session.pause", json!({ "sid": work.sid(), "reason": "hold on" })).await.unwrap();
    assert!(paused["index"].as_u64().is_some(), "{paused}");
    let resumed = rpc(port, "session.resume", json!({ "sid": work.sid() })).await.unwrap();
    assert!(resumed["index"].as_u64().is_some(), "{resumed}");
    let started = Instant::now();
    let stopped = rpc(port, "session.stop", json!({ "sid": work.sid(), "reason": "enough" })).await.unwrap();
    assert!(stopped["index"].as_u64().is_some(), "{stopped}");
    until("stopped", || {
        finished(&work) && registered_status(loader.agent.as_ref(), work.sid()).is_some_and(|s| s.run_state == RunState::Finished)
    })
    .await;
    assert!(started.elapsed() < Duration::from_secs(15), "the command was interrupted, not waited for");
    assert_eq!(work.state().unwrap().outcome, Some(Outcome::Stopped));
    let again = rpc(port, "session.stop", json!({ "sid": work.sid() })).await.unwrap_err();
    assert!(again.to_string().contains("finished"), "{again}");
    loader.shutdown().await;
}

/// `session.work_cards` over kRPC (TODO messagehub-worksession-ui §3.5):
/// only the work the given Turn created, a plan with its TodoList and the
/// card of a bound Todo; `session.worklog` without a Turn is the whole
/// session in commit order (§3.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_cards_and_the_continuous_worklog_over_krpc() {
    let w = World::new();
    let port = free_port();
    let loader = Loader::start(w.env(Llm::new(|_, _| "done".into()), port).await).await.unwrap();
    let parent = new_work(loader.agent.as_ref(), &w, "the conversation").await;
    until("the parent finishes", || finished(&parent)).await;
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    // Work of other drivers: left alone by this loader, still shown.
    let child = |parent: &str, task: &str, title: &str, class: &str| {
        let mut spec = work_spec(&format!("objective of {title}"), "go");
        spec.driver = Some("app:other@alice".into());
        spec.title = Some(title.into());
        spec.class = class.into();
        spec.origin = Some(Origin {
            parent_session: Some(parent.to_string()),
            parent_task: Some(task.into()),
            ..Default::default()
        });
        spec
    };
    let mut created = Vec::new();
    for (task, title, class) in [("t-ui-1", "Simple", "work"), ("t-ui-2", "Elsewhere", "work"), ("t-ui-1", "Plan", "plan_simple")] {
        created.push(
            create_session(&w.root.join("sessions"), child(parent.sid(), task, title, class), loader.agent.as_ref(), WHO, w.channels().as_ref())
                .await
                .unwrap(),
        );
    }
    let plan = &created[2];
    let todo = create_session(&w.root.join("sessions"), child(plan.sid(), "t-plan-1", "Survey", "work"), loader.agent.as_ref(), WHO, w.channels().as_ref())
        .await
        .unwrap();
    let list = json!({
        "schema": "opendan.todo_list/1", "next_seq": 3, "rev": 2,
        "todos": [
            { "todo_id": "T01", "seq": 1, "rev": 1, "status": "running", "mode": "serial", "title": "Survey",
              "content": "x", "created_at_ms": 1, "updated_at_ms": 1,
              "attempts": [{ "attempt": 1, "mode": "serial", "child_sid": todo.sid(), "status": "running", "started_at_ms": 1 }] },
            { "todo_id": "T02", "seq": 2, "rev": 1, "status": "pending", "mode": "serial", "title": "Verify",
              "content": "x", "created_at_ms": 1, "updated_at_ms": 1 }
        ]
    });
    std::fs::write(plan.state_dir().join("todos.json"), list.to_string()).unwrap();

    let cards = rpc(port, "session.work_cards", json!({ "sid": parent.sid(), "task_id": "t-ui-1" })).await.unwrap();
    assert_eq!((cards["agent_did"].as_str(), cards["task_id"].as_str()), (Some(AGENT), Some("t-ui-1")));
    let sessions = cards["sessions"].as_array().unwrap();
    let titles: Vec<&str> = sessions.iter().map(|c| c["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Simple", "Plan"]);
    assert_eq!((sessions[0]["shape"].as_str(), sessions[0]["state"].as_str()), (Some("simple"), Some("queued")));
    assert_eq!(sessions[1]["shape"], "plan");
    let todos = &sessions[1]["todos"];
    assert_eq!((todos["total"].as_u64(), todos["done"].as_u64()), (Some(2), Some(0)));
    assert_eq!(todos["items"][0]["card"]["session_id"], todo.sid());
    assert_eq!(todos["items"][0]["card"]["title"], "Survey");
    assert!(todos["items"][1]["card"].is_null());
    assert_eq!(cards["complete"], false);
    let err = rpc(port, "session.work_cards", json!({ "sid": parent.sid() })).await.unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err:?}");

    // The whole worklog of a finished session: across Turns, records
    // without a Turn included, no Turn task.
    let log = rpc(port, "session.worklog", json!({ "sid": parent.sid() })).await.unwrap();
    assert!(log["turn"].is_null() && log["task_id"].is_null(), "{log}");
    assert_eq!(log["complete"], true);
    let entries = log["entries"].as_array().unwrap();
    assert_eq!(entries[0]["t"], "created");
    assert!(entries.iter().any(|e| e["t"] == "turn_ended"));
    let seqs: Vec<u64> = entries.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] + 1 == w[1]), "{seqs:?}");
    let running = rpc(port, "session.worklog", json!({ "sid": created[0].sid() })).await.unwrap();
    assert_eq!(running["complete"], false);
    loader.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_api_routes_metadata_and_rpc_with_the_same_access_policy() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    for app_id in [None, Some("jarvis.alice")] {
        let w = World::new();
        let port = free_port();
        let mut env = w.env(Llm::new(|_, _| "unused".into()), port).await;
        std::fs::write(w.root.join("index.html"), "<html>WebUI</html>").unwrap();
        env.web_dir = Some(w.root.clone());
        if let Some(app_id) = app_id {
            env.access = Access::Zone {
                owner: "bob".into(),
                app_id: app_id.into(),
                trust_loopback: false,
                zone_host: Some("test.buckyos.io".into()),
            };
        }
        let loader = Loader::start(env).await.unwrap();
        until("service answers", || {
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        }).await;
        let agent_path = format!("/kapi/{}", loader.agent.agent_id());
        for path in [WEB_API_PATH, SERVICE_PATH, agent_path.as_str()] {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            stream.write_all(format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            ).as_bytes()).await.unwrap();
            let mut response = String::new();
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut response))
                .await.unwrap().unwrap();
            let (headers, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("HTTP/1.1 200"), "{response}");
            assert!(headers.contains("application/json"), "{headers}");
            assert_eq!(serde_json::from_str::<Value>(body).unwrap(), json!({ "app_id": app_id }));
            let result = KrpcTransport::new(&format!("http://127.0.0.1:{port}{path}"), None)
                .call("loader.status", json!({})).await;
            if app_id.is_some() {
                assert!(result.unwrap_err().to_string().contains("a session token is required"));
            } else {
                assert_eq!(result.unwrap()["agent_did"], AGENT);
            }
            if app_id.is_some() {
                let cards = KrpcTransport::new(&format!("http://127.0.0.1:{port}{path}"), None)
                    .call("session.work_cards", json!({ "sid": "s", "task_id": "t" })).await;
                assert!(cards.unwrap_err().to_string().contains("a session token is required"));
            }
        }
        loader.shutdown().await;
    }
}

/// M1 / E4: what a caller without the AgentRoot gets over real kRPC is what
/// the files say; signed writes arrive; a driver's writes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_state_over_krpc_matches_the_files() {
    let w = World::new();
    std::fs::write(w.root.join("role.md"), "I am Jarvis.").unwrap();
    std::fs::create_dir_all(w.root.join("behaviors")).unwrap();
    std::fs::write(
        w.root.join("behaviors/plan.toml"),
        "[meta]\nobjective = \"plan\"\n[prompt]\nsystem = \"PLAN\"\n",
    )
    .unwrap();
    let port = free_port();
    let llm = Llm::new(|req, _| format!("echo: {}", last_user(req).len()));
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    let done = new_work(loader.agent.as_ref(), &w, "a finished one").await;
    until("work session finishes", || finished(&done)).await;
    until("service answers", || {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    })
    .await;

    let endpoint = format!("http://127.0.0.1:{port}{SERVICE_PATH}");
    let remote = connect(
        AGENT,
        "app:tool@alice",
        ConnectOptions {
            hint: Some(StateLocator::Krpc { endpoint: endpoint.clone() }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let local = fs_agent(&w);
    assert!(remote.agent_root().is_none());
    assert_eq!(remote.agent_id(), local.agent_id());
    // A finished session's record is written once more as its runner lets it go.
    until("the record over kRPC is the file's", || {
        futures_block(remote.sessions().lookup(done.sid())).unwrap() == futures_block(local.sessions().lookup(done.sid())).unwrap()
    })
    .await;
    assert_eq!(remote.sessions().lookup("no-such").await.unwrap(), None);
    let q = RegistryQuery {
        driver: Some(WHO.into()),
        ..Default::default()
    };
    assert_eq!(
        remote.sessions().query(&q).await.unwrap(),
        local.sessions().query(&q).await.unwrap()
    );
    assert_eq!(
        remote.behaviors().identity().await.unwrap(),
        local.behaviors().identity().await.unwrap()
    );
    assert_eq!(
        remote.behaviors().list().await.unwrap(),
        local.behaviors().list().await.unwrap()
    );
    assert_eq!(
        remote.behaviors().get("plan").await.unwrap(),
        local.behaviors().get("plan").await.unwrap()
    );
    assert_eq!(
        remote.behaviors().revision().await.unwrap(),
        local.behaviors().revision().await.unwrap()
    );
    assert_eq!(
        remote.perception().last_seq(done.sid()).await.unwrap(),
        local.perception().last_seq(done.sid()).await.unwrap()
    );
    assert_eq!(
        remote.artifacts().list().await.unwrap(),
        local.artifacts().list().await.unwrap()
    );
    assert_eq!(
        remote.activity().active(None, 10).await.unwrap(),
        local.activity().active(None, 10).await.unwrap()
    );
    // The wrong agent is refused when connecting.
    let wrong = connect(
        "did:bns:someone.else",
        WHO,
        ConnectOptions {
            hint: Some(StateLocator::Krpc { endpoint }),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(wrong, Err(OpenDanError::NotFound(_))));

    // A signed write: a session created through the service by a caller
    // that only has the remote client; then an input posted to it.
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("remote dialogue");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some("local:remote-test".into());
    let ui = create_session(
        &w.root.join("sessions"),
        spec,
        remote.as_ref(),
        WHO,
        w.channels().as_ref(),
    )
    .await
    .unwrap();
    assert!(local.sessions().lookup(ui.sid()).await.unwrap().is_some());
    remote
        .sessions()
        .post_input(ui.sid(), &PostedInput::text(WHO, AGENT, "hello over krpc").unwrap())
        .await
        .unwrap();
    until("posted input is answered", || {
        ui.state().map(|s| s.turns_completed == 1).unwrap_or(false)
    })
    .await;
    // Errors keep their kind across the wire.
    let err = remote
        .sessions()
        .post_input(done.sid(), &PostedInput::text(WHO, AGENT, "late").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, OpenDanError::SessionFinished(ref sid) if sid == done.sid()), "{err:?}");
    let err = rpc(port, "sessions.lookup", json!({})).await.unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err:?}");
    // A driver's writes need the AgentRoot.
    assert!(remote.locks().is_held(libopendan::memory::CONSOLIDATION_LEASE).is_err());

    // Observation and the Loader's own state.
    let view = rpc(port, "session.read", json!({ "sid": ui.sid() })).await.unwrap();
    assert_eq!(view["entry"]["session_id"], ui.sid());
    assert_eq!(view["state"]["turns_completed"], 1);
    assert_eq!(view["config"]["session"]["class"], "ui");
    assert!(view["worklog"].as_array().is_some_and(|w| !w.is_empty()));
    let page = rpc(port, "session.worklog", json!({ "sid": ui.sid(), "turn": 1, "limit": 2 })).await.unwrap();
    assert_eq!(page["agent_did"], AGENT);
    assert_eq!(page["session_id"], ui.sid());
    assert_eq!(page["complete"], true);
    let alias = KrpcTransport::new(&format!("http://127.0.0.1:{port}/kapi/{}", local.agent_id()), None)
        .call("session.worklog", json!({ "sid": ui.sid(), "turn": 1, "limit": 2 })).await.unwrap();
    assert_eq!(alias["entries"], page["entries"]);
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    assert!(page["entries"].as_array().unwrap().iter().all(|e| e["turn"] == 1));
    let older = rpc(port, "session.worklog", json!({ "sid": ui.sid(), "turn": 1, "before": page["next_before"] })).await.unwrap();
    assert!(!older["entries"].as_array().unwrap().is_empty());
    let tail = rpc(port, "session.worklog", json!({ "sid": ui.sid(), "turn": 1, "after": page["next_after"] })).await.unwrap();
    assert!(tail["entries"].as_array().unwrap().is_empty());
    assert!(rpc(port, "session.worklog", json!({ "sid": ui.sid(), "turn": 0 })).await.is_err());
    assert_eq!(view["hosted"]["session_id"], ui.sid());
    let status = rpc(port, "loader.status", json!({})).await.unwrap();
    assert_eq!(status["agent_did"], AGENT);
    assert!(status["authentication"].is_null());
    let modules: HashMap<String, bool> = status["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["name"].as_str().unwrap().to_string(), m["running"].as_bool().unwrap()))
        .collect();
    assert_eq!(modules["ui"], true);
    assert_eq!(modules["agent_state_service"], true);
    assert!(!modules.contains_key("self_check"), "self_check is retired");
    assert!(status["hosted"].as_array().unwrap().iter().any(|h| h["session_id"] == ui.sid()));

    // The home page: profile card, usage by model, conversations. Without
    // a nickname the agent is called by its user name.
    let profile = rpc(port, "agent.profile", json!({})).await.unwrap();
    assert_eq!(profile["agent_did"], AGENT);
    assert_eq!(profile["display_name"], "jarvis");
    assert_eq!(profile["owner_did"], BOB);
    assert!(profile["desktop_url"].is_null());
    let profile = rpc(port, "agent.profile_set", json!({ "display_name": "Jay", "bio": "at your service" }))
        .await
        .unwrap();
    assert_eq!(profile["display_name"], "Jay");
    assert_eq!(rpc(port, "agent.profile", json!({})).await.unwrap()["bio"], "at your service");
    // The profile is the agent's record in the zone, not a file of the AgentRoot.
    let stored = records::profile(w.records.as_ref()).await.unwrap();
    assert_eq!(stored.display_name.as_deref(), Some("Jay"));
    assert!(!w.root.join(".meta/profile.json").exists());
    let err = rpc(port, "agent.profile_set", json!({ "avatar": "file:///etc/passwd" })).await.unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err:?}");
    let usage = rpc(port, "usage.models", json!({})).await.unwrap();
    let models = usage["models"].as_array().unwrap();
    assert!(!models.is_empty(), "{usage}");
    let recorded: u64 = models.iter().map(|m| m["all"]["total"].as_u64().unwrap()).sum();
    let counted: u64 = [&done, &ui].iter().map(|sd| sd.usage().unwrap().iter().map(|r| r.total_tokens).sum::<u64>()).sum();
    assert!(recorded >= 30 && recorded == counted, "{usage}");
    assert_eq!(models[0]["hour"], models[0]["all"]);
    assert!(rpc(port, "ui.bindings", json!({})).await.unwrap().is_array());

    std::fs::create_dir_all(w.root.join("workspace/unregistered")).unwrap();
    assert_eq!(rpc(port, "home.workspaces", json!({})).await.unwrap(), json!([]));
    let external = tempfile::tempdir().unwrap();
    let directory = external.path().join("project");
    let registered = rpc(port, "workspaces.create", json!({ "request": {
        "operation_id": "homepage-workspace", "name": "Shared project", "description": "A long-term project",
        "location": { "runtime_id": "local", "directory": directory }, "usage": "collaborative"
    }})).await.unwrap();
    let canonical = std::fs::canonicalize(&directory).unwrap();
    let workspace_id = registered["workspace_id"].as_str().unwrap();
    let annotated = rpc(port, "workspaces.update", json!({
        "workspace_id": workspace_id, "request": { "expected_revision": registered["revision"], "private_notes": "private cognition" }
    })).await.unwrap();
    let archived = rpc(port, "workspaces.archive", json!({
        "workspace_id": workspace_id, "expected_revision": annotated["revision"]
    })).await.unwrap();
    assert_eq!(archived["lifecycle"], "archived");
    std::fs::rename(&directory, external.path().join("moved-project")).unwrap();
    let checked = rpc(port, "workspaces.check", json!({ "workspace_id": workspace_id })).await.unwrap();
    assert_eq!(checked["availability"], "missing");
    let known = rpc(port, "home.workspaces", json!({})).await.unwrap();
    assert_eq!(known.as_array().unwrap().len(), 1);
    assert_eq!(known[0]["workspace_id"], workspace_id);
    assert_eq!(known[0]["location"]["directory"], json!(canonical));
    assert_eq!(known[0]["lifecycle"], "archived");
    assert_eq!(known[0]["availability"], "missing");
    assert!(known[0]["last_error"].is_string());
    assert!(known[0].get("private_notes").is_none());
    assert!(external.path().join("moved-project").exists());

    // The three operations are records on the session's bus.
    let posted = rpc(port, "session.post", json!({ "sid": ui.sid(), "text": "second" })).await.unwrap();
    assert!(posted["key"].as_str().unwrap().starts_with("cymsg:"));
    until("second turn", || {
        ui.state().map(|s| s.turns_completed == 2).unwrap_or(false)
            && registered_status(loader.agent.as_ref(), ui.sid()).is_some_and(|s| !s.turn_open)
    })
    .await;
    // A conversation is not stopped: a stop is for the message in
    // progress, and its context is dropped by archiving it.
    let refused = rpc(port, "session.stop", json!({ "sid": ui.sid(), "reason": "enough" })).await.unwrap_err();
    assert!(refused.to_string().contains("ui.rotate"), "{refused}");
    let idle = rpc(port, "session.stop", json!({ "sid": ui.sid(), "scope": "turn" })).await.unwrap_err();
    assert!(idle.to_string().contains("no Turn in progress"), "{idle}");
    assert!(!ui.state().unwrap().is_finished());
    let archived = rpc(port, "ui.rotate", json!({ "sid": ui.sid() })).await.unwrap();
    assert_eq!(archived, json!({ "archived": ui.sid(), "finished": true }));
    until("stopped", || {
        ui.state().map(|s| s.outcome == Some(Outcome::Stopped)).unwrap_or(false)
    })
    .await;
    let err = rpc(port, "session.decide", json!({ "sid": done.sid(), "decision": "maybe" }))
        .await
        .unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err:?}");
    loader.shutdown().await;
}

fn ui_entry(agent: &dyn AgentStateClient, route: &str) -> Option<RegistryEntry> {
    let entries = futures_block(agent.sessions().query(&RegistryQuery {
        kind: Some(SessionKind::Ui),
        ..Default::default()
    }))
    .unwrap();
    entries.into_iter().find(|e| e.route_key.as_deref() == Some(route))
}

fn read_worklog_len(sd: &SessionDir) -> usize {
    sd.worklog().read_all_for_audit().map(|w| w.len()).unwrap_or(0)
}

fn futures_block<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

/// M3: a message in an inbox creates the UI session bound to that inbox;
/// the reply goes back to the speaker, in the same conversation, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbox_messages_are_answered_along_the_same_conversation() {
    let w = World::new();
    let llm = Llm::new(|req, n| format!("answer {n} to: {}", last_user(req).contains("hello")));
    let port = free_port();
    let loader = Loader::start(w.env(llm.clone(), port).await).await.unwrap();
    let session = format!("dm:{BOB}");
    let first = chat(BOB, "hello jarvis");
    let first_id = msg_key(&first);
    w.mail.deliver(&session, first);
    until("first reply", || w.mail.sent().len() == 1).await;
    let (key, reply) = w.mail.sent().remove(0);
    assert_eq!(reply.from, parse_did(AGENT).unwrap());
    assert_eq!(reply.to, vec![parse_did(BOB).unwrap()]);
    assert_eq!(reply.kind, MsgObjKind::Chat);
    assert_eq!(reply.to_session, None, "the default conversation is not named");
    assert_eq!(reply.thread.reply_to.as_ref().unwrap().to_string(), first_id);
    assert!(reply.content.content.starts_with("answer 0"), "{}", reply.content.content);
    assert_eq!(
        reply.meta.get("delivery_failure_notice").and_then(Value::as_str),
        Some("(the reply could not be delivered)")
    );
    assert_eq!(w.mail.unread(), 0, "acknowledged after it was posted");

    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).expect("ui session of the inbox");
    let (date, number) = entry.session_id.strip_prefix("ui-").unwrap().split_once('-').unwrap();
    assert_eq!(date.len(), 8);
    assert!(date.bytes().all(|b| b.is_ascii_digit()));
    assert!(number.parse::<u64>().unwrap() > 0);
    assert_eq!(
        loader.agent.sessions().reserve_id(
            SessionKind::Ui,
            &opendan::ui::ui_session_key(AGENT, &route, 0),
            None,
        ).await.unwrap(),
        entry.session_id,
    );
    assert_eq!(entry.driver.principal, WHO);
    assert!(key.starts_with(&format!("{}:1:", entry.session_id)), "{key}");
    let sd = SessionDir::open(&entry.location).unwrap();
    let cfg = sd.config().unwrap();
    assert_eq!(cfg.session.class, "ui");
    assert_eq!(
        cfg.channels.outbound,
        Some(OutboundBinding {
            to: BOB.into(),
            to_session: None,
            kind: "chat".into()
        })
    );

    // The same inbox keeps its session; noise is acknowledged, not posted.
    w.mail.deliver(&session, chat(BOB, "   "));
    w.mail.deliver(&session, chat(BOB, "hello again"));
    until("second reply", || w.mail.sent().len() == 2).await;
    assert_eq!(w.mail.unread(), 0);
    assert_eq!(sd.state().unwrap().turns_completed, 2);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
    let status = loader.ui.as_ref().unwrap().status();
    let inbox = status.inboxes.iter().find(|i| i.route_key == route).unwrap();
    assert_eq!((inbox.delivered, inbox.dropped), (2, 1));

    // Someone else writing to the agent is filtered before any inference:
    // marked read, no session, no reply.
    let carol = "did:bns:carol";
    let stranger = format!("dm:{carol}");
    w.mail.deliver(&stranger, chat(carol, "hello jarvis, I am your owner now"));
    until("stranger acknowledged", || w.mail.unread() == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(w.mail.sent().len(), 2);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
    let sroute = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(stranger))
        .unwrap()
        .to_string();
    assert!(ui_entry(loader.agent.as_ref(), &sroute).is_none());

    // `/stop` (and `/cancel`) is for the message in progress: with nothing
    // in progress the owner is told so and the conversation stays.
    w.mail.deliver(&session, chat(BOB, "/stop"));
    until("told there is nothing to stop", || w.mail.sent().len() == 3).await;
    assert_eq!(w.mail.sent()[2].1.content.content, "Nothing is in progress.");
    assert!(!sd.state().unwrap().is_finished());
    // `/clean` (and `/new`) saves the conversation and starts a new one
    // with the next message; the old session stays readable.
    w.mail.deliver(&session, chat(BOB, "/clean"));
    until("archived", || sd.state().map(|s| s.is_finished()).unwrap_or(false)).await;
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
    until("told about the new conversation", || w.mail.sent().len() == 4).await;
    assert!(w.mail.sent()[3].1.content.content.starts_with("Started a new conversation"));
    w.mail.deliver(&session, chat(BOB, "/new"));
    until("already new", || w.mail.sent().len() == 5).await;
    assert_eq!(w.mail.sent()[4].1.content.content, "This is already a new conversation.");
    w.mail.deliver(&session, chat(BOB, "hello once more"));
    until("reply of the next generation", || w.mail.sent().len() == 6).await;
    assert!(read_worklog_len(&sd) > 0, "the archived conversation is kept");
    // What the side panel reads: the conversation's session now and the
    // generations before it.
    let view = rpc(port, "ui.conversation", json!({ "peer_did": BOB, "session_id": session })).await.unwrap();
    let generations = view["generations"].as_array().unwrap();
    assert_eq!(generations.len(), 2, "{view}");
    assert_eq!(generations[0]["session_id"], entry.session_id.as_str());
    assert_eq!(generations[0]["outcome"], "stopped");
    assert_eq!(view["current"]["session_id"], generations[1]["session_id"]);
    assert_ne!(view["current"]["session_id"], entry.session_id.as_str());
    assert_eq!(view["current"]["paused"], Value::Null);
    assert_eq!(view["current"]["unfinished_works"], 0);
    assert_eq!(view["current"]["prompt"]["current"], true, "{view}");
    let tree = rpc(port, "session.work_tree", json!({ "sid": view["current"]["session_id"], "unfinished_only": true })).await.unwrap();
    assert_eq!((tree["complete"].as_bool(), tree["sessions"].as_array().map(Vec::len)), (Some(true), Some(0)), "{tree}");
    let all: Vec<RegistryEntry> = loader
        .agent
        .sessions()
        .query(&RegistryQuery::default())
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.route_key.as_deref() == Some(route.as_str()))
        .collect();
    assert_eq!(all.len(), 2);
    loader.shutdown().await;
}

/// SC11 over the tunnel: the host restarts while a message is worked on —
/// the conversation is interrupted, not continued; a message sent meanwhile
/// waits and the owner is told once how to go on; `/resume` continues the
/// interrupted Turn, then the waiting message is answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_interrupts_the_message_in_progress_until_resume() {
    let w = World::new();
    let llm = Llm::new(move |req, n| {
        if last_user(req).contains("the next thing") {
            return "answer to the next thing".to_string();
        }
        // The host goes away while this command runs.
        if n == 0 {
            return "TOOL:sleep 4".to_string();
        }
        "answer to the long job".to_string()
    });
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "a long job"));
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    until("the command is running", || llm.calls.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    loader.shutdown().await;
    assert!(sd.state().unwrap().open_turn.is_some());
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);

    let loader = w.start(llm.clone()).await;
    until("interrupted", || {
        sd.state().is_ok_and(|s| s.paused.as_ref().is_some_and(|p| p.source == PauseSource::Interrupted))
    })
    .await;
    let calls = llm.calls.load(Ordering::SeqCst);
    let sent = w.mail.sent().len();
    w.mail.deliver(&session, chat(BOB, "the next thing"));
    until("told about the interruption", || w.mail.sent().len() == sent + 1).await;
    assert!(w.mail.sent()[sent].1.content.content.contains("/resume"), "{:?}", w.mail.sent()[sent].1.content.content);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), calls, "nothing is inferred while interrupted");
    assert_eq!(w.mail.sent().len(), sent + 1, "told once");

    w.mail.deliver(&session, chat(BOB, "/resume"));
    until("both answered", || {
        let texts: Vec<String> = w.mail.sent().into_iter().map(|(_, m)| m.content.content).collect();
        texts.iter().any(|t| t.contains("answer to the long job")) && texts.iter().any(|t| t.contains("answer to the next thing"))
    })
    .await;
    assert!(w.mail.sent().iter().any(|(_, m)| m.content.content == "Resumed 1 interrupted session(s)."));
    let st = sd.state().unwrap();
    assert!(st.paused.is_none() && st.open_turn.is_none() && !st.is_finished(), "{st:?}");
    loader.shutdown().await;
}

/// A Turn that answers a message and fails tells the speaker so; the next
/// message is answered normally.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_turn_is_reported_to_the_speaker() {
    let w = World::new();
    let llm = Llm::new(|_, _| "fine now".to_string());
    llm.failures.store(1, Ordering::SeqCst);
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    let first = chat(BOB, "hello");
    let first_id = msg_key(&first);
    w.mail.deliver(&session, first);
    until("failure notice", || w.mail.sent().len() == 1).await;
    let (_, notice) = w.mail.sent().remove(0);
    assert_eq!(notice.to, vec![parse_did(BOB).unwrap()]);
    assert_eq!(notice.content.content, "(something went wrong)");
    assert_eq!(notice.thread.reply_to.as_ref().unwrap().to_string(), first_id);
    assert_eq!(notice.meta.get("delivery_failure_fallback"), Some(&Value::Bool(true)));
    w.mail.deliver(&session, chat(BOB, "again"));
    until("reply", || w.mail.sent().len() == 2).await;
    assert_eq!(w.mail.sent().remove(1).1.content.content, "fine now");
    loader.shutdown().await;
}

/// AI Center Quota PRD §9.6: a Turn refused under the owner's AI quota tells
/// the owner where to look in a direct chat, and fails without retrying.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_quota_refusal_tells_the_owner_where_to_look() {
    let w = World::new();
    std::fs::write(
        w.root.join("i18n/en.toml"),
        "[outbound]\nturn_failed = \"(something went wrong)\"\nai_quota_limited = \"(limited)\"\nai_quota_limited_owner = \"(your quota)\"\n",
    )
    .unwrap();
    let llm = Llm::new(|_, _| "fine now".to_string());
    *llm.refusal.lock().unwrap() = Some(agent_tool::xllm::REFUSED_QUOTA_EXHAUSTED);
    llm.failures.store(1, Ordering::SeqCst);
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "hello"));
    until("refusal notice", || w.mail.sent().len() == 1).await;
    assert_eq!(w.mail.sent().remove(0).1.content.content, "(your quota)");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "a refusal is not retried");
    loader.shutdown().await;
}

/// M3: the Loader dies after the reply was committed and before msg-center
/// took it. The next process sends the stored message — the same ObjId
/// under the same key — and nothing is inferred or sent twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_committed_reply_survives_a_restart_and_is_sent_once() {
    let w = World::new();
    w.mail.unreachable.store(true, Ordering::SeqCst);
    let llm = Llm::new(|_, n| format!("answer {n}"));
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "hello"));
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    until("reply committed", || {
        ui_entry(loader.agent.as_ref(), &route)
            .and_then(|e| SessionDir::open(&e.location).ok())
            .and_then(|sd| sd.state().ok())
            .is_some_and(|s| s.outbox.len() == 1 && s.outbox[0].attempts >= 1)
    })
    .await;
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    let stored = sd.state().unwrap().outbox[0].clone();
    assert_eq!(stored.status, OutboxStatus::Pending);
    assert_eq!(w.mail.unread(), 0, "the input was consumed and acknowledged");
    assert!(w.mail.sent().is_empty());
    // Leaving the Loader does not stop the session.
    loader.shutdown().await;
    let st = sd.state().unwrap();
    assert!(!st.is_finished() && st.outcome.is_none());

    w.mail.unreachable.store(false, Ordering::SeqCst);
    let loader = w.start(llm.clone()).await;
    until("stored reply sent", || w.mail.sent().len() == 1).await;
    until("marked sent", || {
        sd.state().map(|s| s.outbox[0].status == OutboxStatus::Sent).unwrap_or(false)
    })
    .await;
    let (key, msg) = w.mail.sent().remove(0);
    assert_eq!(key, stored.key);
    assert_eq!(msg, stored.msg);
    assert_eq!(msg_key(&msg), msg_key(&stored.msg));
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "no second inference");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(w.mail.sent().len(), 1);
    loader.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authentication_failure_keeps_the_inbox_unread_until_recovery() {
    let w = World::new();
    w.mail.authentication_unavailable.store(true, Ordering::SeqCst);
    w.mail.deliver(&format!("dm:{BOB}"), chat(BOB, "message during auth failure"));
    let llm = Llm::new(|_, _| "recovered reply".into());
    let loader = w.start(llm.clone()).await;
    let ui = loader.ui.as_ref().unwrap();
    until("authentication unavailable is visible", || {
        ui.status().last_error.as_deref().is_some_and(|error| error.contains("authentication unavailable"))
    }).await;
    assert_eq!(w.mail.unread(), 1);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    assert!(w.mail.sent().is_empty());
    assert!(loader.agent.sessions().query(&RegistryQuery::default()).await.unwrap().is_empty());
    loader.shutdown().await;

    w.mail.authentication_unavailable.store(false, Ordering::SeqCst);
    let loader = w.start(llm.clone()).await;
    until("backlog reply delivered", || w.mail.sent().len() == 1).await;
    assert_eq!(w.mail.unread(), 0);
    assert_eq!(w.mail.sent()[0].1.content.content, "recovered reply");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    loader.shutdown().await;
    let loader = w.start(llm.clone()).await;
    loader.ui.as_ref().unwrap().scan().await.unwrap();
    assert_eq!(w.mail.sent().len(), 1);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    loader.shutdown().await;
}

/// M3: a session whose queue is full holds its inbox: the record is not
/// acknowledged upstream and is delivered once there is room.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_full_session_queue_holds_the_inbox() {
    let w = World::new();
    let gate = Arc::new(AtomicBool::new(false));
    let g = gate.clone();
    let llm = Llm::new(move |_, n| {
        while !g.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        format!("answer {n}")
    });
    let loader = w.start(llm).await;
    let session = format!("dm:{BOB}");
    let total = MAX_PENDING_INPUTS + 12;
    for i in 0..total {
        w.mail.deliver(&session, chat(BOB, &format!("message {i}")));
    }
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    let ui = loader.ui.clone().unwrap();
    until("inbox held", || {
        ui.status()
            .inboxes
            .iter()
            .any(|i| i.route_key == route && i.held.as_deref().is_some_and(|h| h.contains("input_full")))
    })
    .await;
    assert!(w.mail.unread() > 0, "records beyond the queue stay unread upstream");
    assert!(w.mail.unread() < total);
    gate.store(true, Ordering::SeqCst);
    until("everything delivered", || w.mail.unread() == 0).await;
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    until("everything consumed", || {
        let delivered = ui.status().inboxes.iter().find(|i| i.route_key == route).map(|i| i.delivered);
        delivered == Some(total as u64)
            && sd.state().map(|s| s.open_turn.is_none() && s.run_state == RunState::Waiting).unwrap_or(false)
    })
    .await;
    assert!(!w.mail.sent().is_empty());
    loader.shutdown().await;
}

/// M3: a work session handed out by a UI session is hosted like any other
/// session of this driver; its result enters the parent as an input and the
/// parent's answer leaves along the parent's conversation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sub_session_reports_through_its_ui_parent() {
    let w = World::new();
    let llm = Llm::new(|req, _| {
        let u = last_user(req);
        if u.contains("there are 3 files") {
            "the work session says: 3 files".to_string()
        } else if u.contains("count the files") {
            r#"CALL:report {"report": "there are 3 files", "is_end": true, "outcome": "succeeded"}"#.to_string()
        } else {
            "on it".to_string()
        }
    });
    let loader = w.start(llm).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "how many files are there?"));
    until("first reply", || w.mail.sent().len() == 1).await;
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let parent = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let child = libopendan::api::create_sub_session(
        loader.agent.as_ref(),
        WHO,
        &parent.session_id,
        libopendan::api::SubSessionSpec {
            objective: "count the files".into(),
            msgs: vec!["count the files".into()],
            key: Some("k1".into()),
            ..Default::default()
        },
        w.channels().as_ref(),
    )
    .await
    .unwrap();
    until("sub session finishes", || finished(&child)).await;
    assert_eq!(child.config().unwrap().session.driver.principal, WHO);
    // A headless sub session has no message route of its own.
    assert!(child.state().unwrap().outbox.is_empty());
    until("the parent relays the result", || w.mail.sent().len() == 2).await;
    let (_, relay) = w.mail.sent().remove(1);
    assert_eq!(relay.to, vec![parse_did(BOB).unwrap()]);
    assert_eq!(relay.content.content, "the work session says: 3 files");
    loader.shutdown().await;
}

/// A work session whose Turn fails for good ends as failed, and its UI
/// parent hears about it like about any other result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_sub_session_is_reported_to_its_ui_parent() {
    let w = World::new();
    let seen = Arc::new(Mutex::new(String::new()));
    let s2 = seen.clone();
    let llm = Llm::new(move |req, n| {
        if n > 0 {
            *s2.lock().unwrap() = last_user(req);
            "the work could not be done".to_string()
        } else {
            "on it".to_string()
        }
    });
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "count the files"));
    until("first reply", || w.mail.sent().len() == 1).await;
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let parent = ui_entry(loader.agent.as_ref(), &route).unwrap();
    llm.failures.store(1, Ordering::SeqCst);
    let child = libopendan::api::create_sub_session(
        loader.agent.as_ref(),
        WHO,
        &parent.session_id,
        libopendan::api::SubSessionSpec {
            objective: "count the files".into(),
            msgs: vec!["count the files".into()],
            key: Some("k1".into()),
            ..Default::default()
        },
        w.channels().as_ref(),
    )
    .await
    .unwrap();
    until("sub session ends", || finished(&child)).await;
    assert_eq!(child.state().unwrap().outcome, Some(Outcome::Failed));
    until("the parent tells the user", || w.mail.sent().len() == 2).await;
    assert_eq!(w.mail.sent().remove(1).1.content.content, "the work could not be done");
    let told = seen.lock().unwrap().clone();
    assert!(told.contains(child.sid()) && told.contains("provider refused the request"), "{told}");
    loader.shutdown().await;
}

/// The Jarvis package of this repository loads under the new conventions:
/// `agent.toml` passes the Loader's checks, the migrated behaviors parse,
/// and a UI session created from it freezes its entry behavior and the
/// routing sub context.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_jarvis_package_loads_and_answers() {
    let w = World::new();
    std::fs::remove_file(w.root.join("agent.toml")).unwrap();
    std::fs::remove_dir_all(w.root.join("i18n")).unwrap();
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/jarvis_agent/agent");
    let seen = Arc::new(Mutex::new(String::new()));
    let tools = Arc::new(Mutex::new(Vec::<String>::new()));
    let (s2, t2) = (seen.clone(), tools.clone());
    let llm = Llm::new(move |req, _| {
        *t2.lock().unwrap() = req.tool_specs.iter().map(|t| t.name.clone()).collect();
        *s2.lock().unwrap() = req
            .messages
            .iter()
            .filter(|m| m.role == AiRole::System)
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");
        // The UI reply is structured (`[reply]` of chat_route).
        serde_json::json!({ "reply": "你好", "topic": null, "artifacts": [] }).to_string()
    });
    let port = free_port();
    let mut env = w.env(llm, port).await;
    env.package_root = Some(package);
    let loader = Loader::start(env).await.unwrap();
    assert_eq!(loader.config.language, "zh");
    assert_eq!(loader.config.llm_context["provider"]["type"], "buckyos");
    for name in ["chat_route", "work_simple", "explore", "goal", "plan_simple"] {
        let b = loader.agent.behaviors().get(name).await.unwrap().unwrap();
        assert!(b.prompt.system.is_some(), "{name}");
    }
    for gone in ["plan", "do"] {
        assert!(loader.agent.behaviors().get(gone).await.unwrap().is_none(), "no behavior loop `{gone}`");
    }
    // plan_simple: native function calls with an intent on every call and
    // inputs that place the background; it only reads its workspace.
    let plan_simple = loader.agent.behaviors().get("plan_simple").await.unwrap().unwrap();
    assert_eq!(plan_simple.capabilities.workspace, Some(libopendan::protocol::WorkspaceAccess::ReadOnly));
    let entry = plan_simple.to_entry("plan_simple", &loader.config.llm_context, true).unwrap();
    assert!(entry.call_intent);
    assert!(entry.prompt.on_input.as_deref().unwrap().contains("{{ background_context }}"));
    assert!(loader.agent.behaviors().get("task_route").await.unwrap().is_none());
    // Intent analysis: a fork of the UI context offered as a named function.
    let analyze = loader.agent.behaviors().get("analyze_intent").await.unwrap().unwrap();
    assert!(analyze.prompt.system.is_none() && analyze.model.preferred.is_none());
    assert!(analyze.prompt.on_context_switch.as_deref().unwrap().contains("ask_user"));
    let entry = analyze.to_entry("analyze_intent", &loader.config.llm_context, false).unwrap();
    assert_eq!(entry.mode, libopendan::protocol::ContextMode::Fork);
    assert_eq!(entry.tool.as_ref().unwrap().function_name("analyze_intent"), "try_analyze_user_intent");
    assert_eq!(
        entry.tool.unwrap().render_task(&serde_json::json!({ "request": "tidy the docs" })),
        "tidy the docs"
    );
    let simple = loader.agent.behaviors().get("work_simple").await.unwrap().unwrap();
    assert_eq!(simple.capabilities.tool_whitelist.as_deref(), Some(&["group:bash".to_string(), "report".to_string()][..]));
    // The private runtime is registered before anything runs; nothing else
    // is assigned to this agent.
    let runtimes = loader.agent.runtimes().list().await.unwrap();
    let ids: Vec<_> = runtimes.iter().map(|r| (r.runtime_id.as_str(), r.availability)).collect();
    assert_eq!(ids, [("private", libopendan::protocol::RuntimeAvailability::Available)]);
    // Both conversations reply under a contract; the analysis returns its
    // result under its own, listing the questions it asked.
    for name in ["chat_route", "groupchat_route"] {
        let b = loader.agent.behaviors().get(name).await.unwrap().unwrap();
        let reply = b.reply.as_ref().unwrap_or_else(|| panic!("{name} has no [reply]"));
        assert_eq!(reply.schema["required"], serde_json::json!(["reply", "topic", "artifacts"]), "{name}");
        assert_eq!(
            b.capabilities.tool_whitelist.as_deref(),
            Some(
                &["group:bash_core".to_string(), "submit_reply".to_string(), "behavior:analyze_intent".to_string()][..]
            ),
            "{name}"
        );
    }
    let reply = analyze.reply.as_ref().unwrap();
    assert_eq!(reply.asks_field.as_deref(), Some("clarifications"), "filled by the host");
    assert_eq!(reply.contract, Some(libopendan::protocol::ReplyContract::IntentAnalysis));
    assert!(reply.schema["properties"].get("clarifications").is_none());
    assert_eq!(loader.config.ui_rule("msg.group").unwrap().session_class, "group");
    let group = SessionTemplate::load("group", loader.agent.agent_root()).unwrap();
    assert_eq!(group.default_behavior.as_deref(), Some("groupchat_route"));
    assert_eq!(
        std::fs::read_to_string(w.root.join(".meta/role_supplement.md")).unwrap(),
        "Your name is jarvis (account `jarvis`, DID `did:bns:jarvis.alice`). Your owner is `bob` (DID `did:bns:bob`).\n",
        "without a profile name or a supplement only the account name, DIDs and owner are added"
    );
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "你好 Jarvis"));
    until("reply", || w.mail.sent().len() == 1).await;
    let (_, reply) = w.mail.sent().remove(0);
    assert_eq!(reply.content.content, "你好");
    assert!(reply.meta["delivery_failure_notice"].as_str().unwrap().contains("没能送达"));
    let system = seen.lock().unwrap().clone();
    assert!(system.contains("primary personal Agent"), "identity is part of the system text");
    assert!(system.contains("Your name is jarvis"), "{system}");
    assert!(!system.contains("You are Jarvis"), "the template does not name the instance: {system}");
    assert!(system.contains("This is a one-to-one chat with your owner (bob, DID did:bns:bob)"), "{system}");
    let (_, commands) = system.split_once("## Commands\n").expect("the frozen cmd_skill is in the prompt");
    for action in ["xcall owner.cal add --params", "xcall owner.cal list --params"] {
        assert!(commands.contains(action), "missing calendar command: {action}");
    }
    assert!(!system.contains("requires: an OpenDAN UI session"), "front matter is not part of the manual: {system}");
    assert!(!system.contains("<<"), "{system}");
    let mut tools = tools.lock().unwrap().clone();
    tools.sort();
    // Without the task bridge (a Unix socket) a session follows its commands with `cancel_task`.
    let expected: &[&str] = if cfg!(unix) {
        &["edit_file", "read", "shell", "submit_reply", "try_analyze_user_intent", "write_file"]
    } else {
        &["cancel_task", "edit_file", "read", "shell", "submit_reply", "try_analyze_user_intent", "write_file"]
    };
    assert_eq!(tools, expected);
    assert_eq!(cfg_max_tokens(&loader.config.llm_context), Some(8192));
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let cfg = SessionDir::open(&entry.location).unwrap().config().unwrap();
    assert_eq!(cfg.prompt.behavior.as_deref(), Some("chat_route"));
    cfg.behaviors().unwrap();
    let frozen = cfg.prompt.frozen.unwrap();
    assert!(frozen.behaviors.contains_key("chat_route") && frozen.behaviors.contains_key("analyze_intent"));
    assert_eq!(frozen.cmd_skills.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["ui_core"]);
    assert_eq!(cfg.session.owner.as_deref(), Some(BOB));
    assert!(cfg.session.objective.starts_with("The long-lived conversation of this agent"), "{}", cfg.session.objective);
    // A work session of the package is a plan session by default.
    let template = SessionTemplate::load("work", loader.agent.agent_root()).unwrap();
    assert_eq!(template.default_behavior.as_deref(), Some("plan_simple"));
    // Skills of the package (skills system TODO): three kinds in the directory layout, a plan that only chooses
    // manuals, the owner's methods over the service, and an import that starts a draft-writing work and installs nothing.
    let plan = loader.agent.behaviors().get("plan_simple").await.unwrap().unwrap();
    assert_eq!(plan.capabilities.cmd_skills, CmdSkillsUse::Candidates);
    assert!(loader.agent.behaviors().get("skill_import").await.unwrap().is_some());
    assert!(libopendan::goal::defaults(loader.agent.agent_root(), libopendan::goal::GOAL_CLASS).self_improve.enabled);
    let listed = rpc(port, "skills.list", json!({ "kind": "plan" })).await.unwrap();
    let mut plans: Vec<&str> = listed.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    plans.sort();
    assert_eq!(plans, ["plan_content_production", "plan_research", "plan_software_change"]);
    let ui_set = rpc(port, "skills.ui_get", json!({})).await.unwrap();
    assert_eq!((ui_set["names"].clone(), ui_set["source"].clone()), (json!(["ui_core"]), json!("template")));
    assert!(ui_set["available"].as_array().unwrap().contains(&json!("share")), "{ui_set}");
    let status = rpc(port, "skills.status", json!({})).await.unwrap();
    let status_of = |name: &str| status["skills"].as_array().unwrap().iter().find(|s| s["name"] == name).cloned().unwrap_or_else(|| panic!("{name} in {status}"));
    assert_eq!(status_of("share")["commands"]["state"], "ready", "a manual without `commands` is listed too");
    assert!(status_of("aigc_video")["commands"]["commands"].get("ffmpeg").is_some());

    // Install the repository's sample package as the owner: plan first, then what the plan said.
    use base64::Engine;
    let sample = libopendan::skills::read_pikg(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../test/skills_pikg_sample/dapp_dist/hello-skills-0.1.0.pikg")).unwrap();
    let planned = rpc(port, "skills.install_plan", json!({ "package": sample })).await.unwrap();
    assert_eq!((planned["risk"]["level"].clone(), planned["checked_runtime_id"].clone()), (json!("low"), json!("private")), "{planned}");
    assert!(planned.get("runtime_id").is_none() && planned.get("targets").is_none(), "a skill installs nothing on a runtime: {planned}");
    let refused = rpc(port, "skills.install", json!({ "package": sample, "plan_digest": "sha256:other" })).await.unwrap_err();
    assert!(refused.to_string().contains("not the plan of this package"), "{refused}");
    let installed = rpc(port, "skills.install", json!({ "package": sample, "plan_digest": planned["plan_digest"] })).await.unwrap();
    assert_eq!(installed["skills"].as_array().unwrap().len(), 2, "{installed}");
    assert!(w.root.join("cmd_skills/hello/SKILL.md").is_file() && w.root.join("plan_skills/hello_plan/SKILL.md").is_file());
    let off = rpc(port, "skills.set_enabled", json!({ "name": "hello", "enabled": false, "reason": "trying" })).await.unwrap();
    assert_eq!(off["enabled"], false);
    let added = rpc(port, "skills.ui_set", json!({ "names": ["ui_core", "share"] })).await.unwrap();
    assert_eq!(added["source"], "state");
    assert_eq!(SessionTemplate::load("ui", loader.agent.agent_root()).unwrap().default_cmd_skills, ["ui_core", "share"]);
    rpc(port, "skills.remove", json!({ "kind": "work", "name": "hello" })).await.unwrap();
    assert!(!w.root.join("cmd_skills/hello").exists() && !w.root.join("tools/hello").exists());
    assert!(rpc(port, "skills.remove", json!({ "kind": "work", "name": "ui_core" })).await.is_err(), "a package's manual is disabled, not removed");

    // A draft on the AgentRoot is installed by its slug and is gone afterwards.
    let draft = w.root.join("skills_drafts/howto");
    std::fs::create_dir_all(draft.join("cmd_skills/howto")).unwrap();
    std::fs::write(draft.join("cmd_skills/howto/SKILL.md"), "---\nname: howto\ndescription: how to do it\n---\nSteps.").unwrap();
    std::fs::write(draft.join("draft.json"), json!({ "imported_from": { "digest": "sha256:1", "runtime_id": "private", "platform": "linux" }, "risk": "Reads files only." }).to_string()).unwrap();
    let drafts = rpc(port, "skills.drafts", json!({})).await.unwrap();
    assert_eq!(drafts[0]["slug"], "howto");
    let planned = rpc(port, "skills.install_plan", json!({ "draft": "howto" })).await.unwrap();
    let codes: Vec<&str> = planned["risk"]["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"imported_text") && codes.contains(&"author_note"), "{codes:?}");
    let installed = rpc(port, "skills.install", json!({ "draft": "howto", "plan_digest": planned["plan_digest"] })).await.unwrap();
    assert_eq!(installed["skills"][0]["origin"], "imported");
    assert!(!draft.exists());

    let import = rpc(port, "skills.import", json!({ "text": "# How to tidy notes\n1. list them\n2. merge duplicates\n", "name": "tidy_notes", "source": "notes.md" })).await.unwrap();
    let inbox = import["inbox"].as_str().unwrap();
    assert!(std::fs::read_to_string(w.root.join(inbox)).unwrap().contains("merge duplicates"));
    let entry = loader.agent.sessions().lookup(import["session_id"].as_str().unwrap()).await.unwrap().unwrap();
    let cfg = SessionDir::open(&entry.location).unwrap().config().unwrap();
    assert_eq!(cfg.prompt.behavior.as_deref(), Some("skill_import"));
    assert_eq!(cfg.prompt.frozen.as_ref().unwrap().cmd_skills.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["skill_authoring"]);
    let first = serde_json::to_string(&cfg.prompt.initial_inputs).unwrap();
    assert!(first.contains(inbox) && first.contains("tidy_notes"), "{first}");
    assert!(!w.root.join("cmd_skills/tidy_notes").exists(), "an import installs nothing");
    let _ = base64::engine::general_purpose::STANDARD.encode("");
    loader.shutdown().await;
}

/// The rendered requests of a Jarvis UI session, written for prompt review
/// (`target/prompt-review/jarvis-chat-route.txt`) and checked against the
/// structure the review asked for: a greeting (structured reply with a
/// topic), a follow-up where nothing in the background changed, and the end
/// of a work session the conversation started (2.B: an event batch).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes the rendered Jarvis request for manual prompt review"]
async fn jarvis_prompt_review() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let w = World::new();
    std::fs::remove_file(w.root.join("agent.toml"))?;
    std::fs::remove_dir_all(w.root.join("i18n"))?;
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/jarvis_agent/agent");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let llm = Llm::new(move |req, _| {
        if system_text(req).contains("You are a work session for one bounded task") {
            return r#"REPORT:{"report":"已完成 PR #667 的审查，发现 1 项 P2（中等）问题：创建响应丢失时清理逻辑会遗留临时账户。","is_end":true,"outcome":"succeeded"}"#
                .to_string();
        }
        let mut all = captured.lock().unwrap();
        all.push(req.clone());
        match all.len() {
            1 => json!({"reply": "你好！我是 Jarvis，有什么可以帮你？", "topic": {"title": "问候", "tags": ["greeting"]}, "artifacts": []}),
            2 => json!({"reply": "好的，以后我会尽量简短。", "topic": null, "artifacts": []}),
            _ => json!({"reply": "PR #667 审查完成：发现 1 项 P2 问题，详见报告。", "topic": null, "artifacts": []}),
        }
        .to_string()
    });
    let mut env = w.env(llm, free_port()).await;
    env.package_root = Some(package);
    env.options.load_hints = true;
    let loader = Loader::start(env).await?;
    w.mail.deliver(&format!("dm:{BOB}"), chat(BOB, "你好 Jarvis"));
    until("greeting answered", || w.mail.sent().len() == 1).await;
    w.mail.deliver(&format!("dm:{BOB}"), chat(BOB, "谢谢，回答简短一点就好"));
    until("follow-up answered", || w.mail.sent().len() == 2).await;
    // A work session this conversation started ends.
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(format!("dm:{BOB}")))
        .unwrap()
        .to_string();
    let ui = ui_entry(loader.agent.as_ref(), &route).ok_or("no ui session")?;
    libopendan::api::create_sub_session(
        loader.agent.as_ref(),
        WHO,
        &ui.session_id,
        libopendan::api::SubSessionSpec {
            objective: "审查 BuckyOS PR #667 的 diff，报告有证据的缺陷".into(),
            msgs: vec!["go".into()],
            behavior: Some("work_simple".into()),
            report: ReportMode::Final,
            key: Some("review-667".into()),
            ..Default::default()
        },
        w.channels().as_ref(),
    )
    .await?;
    until("work end relayed", || w.mail.sent().len() == 3).await;
    let ui_state = SessionDir::open(w.root.join("sessions").join(&ui.session_id))?.state()?;
    loader.shutdown().await;
    let requests = requests.lock().unwrap();
    if requests.len() < 3 {
        return Err(format!("expected three UI requests, got {}", requests.len()).into());
    }

    // The structure the review asked for.
    let first = &requests[0];
    let system = system_text(first);
    for heading in ["## Identity", "## Current Session Constraints", "## Objective", "## Replying", "## Memory", "## Commands"] {
        assert!(system.contains(heading), "{heading} missing:\n{system}");
    }
    for gone in ["## capabilities", "## cmd_manual", "## runtime_protocol", "## Routing Work", "## Notifications", "session `ui-", "agent-session note"] {
        assert!(!system.contains(gone), "{gone} still there:\n{system}");
    }
    assert!(system.contains("DID `did:bns:jarvis.alice`") && system.contains("DID `did:bns:bob`"), "{system}");
    assert!(first.force_json && first.json_schema.is_some());
    let mut tools: Vec<&str> = first.tool_specs.iter().map(|t| t.name.as_str()).collect();
    tools.sort();
    // Without the task bridge (a Unix socket) a session follows its commands with `cancel_task`.
    let expected: &[&str] = if cfg!(unix) {
        &["edit_file", "read", "shell", "submit_reply", "try_analyze_user_intent", "write_file"]
    } else {
        &["cancel_task", "edit_file", "read", "shell", "submit_reply", "try_analyze_user_intent", "write_file"]
    };
    assert_eq!(tools, expected);
    let input = last_user(first);
    assert!(input.starts_with("<<background_context>>\n- current_time "), "{input}");
    assert!(input.ends_with("<</background_context>>\n你好 Jarvis"), "{input}");
    assert!(!input.contains("<session_input") && !input.contains("<msg"), "{input}");
    // Nothing in the background changed: the message alone.
    assert_eq!(last_user(&requests[1]), "谢谢，回答简短一点就好");
    // The work's end: one background line and the event, one line.
    let event = last_user(&requests[2]);
    assert!(event.contains("finished (succeeded)") && event.contains("Read: agent-session read-session work-"), "{event}");
    assert!(event.contains("<<inputs>>\n<event ") && !event.contains("report:"), "{event}");
    // The topic came from the structured reply.
    assert_eq!(ui_state.topic.title, "问候");

    let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/prompt-review");
    std::fs::create_dir_all(&output_dir)?;
    let output = output_dir.canonicalize()?.join("jarvis-chat-route.txt");
    let mut file = std::fs::File::create(&output)?;
    writeln!(file, "OpenDAN prompt review")?;
    writeln!(file, "Package: src/apps/jarvis_agent/agent")?;
    writeln!(file, "Scenario: private chat / chat_route / 你好 Jarvis → follow-up → a work session ends")?;
    writeln!(file, "Boundary: LlmClient::infer (scripted response, no model call)")?;
    for (round, req) in requests.iter().enumerate() {
        writeln!(file, "\n===== Request {}: settings =====", round + 1)?;
        let settings = json!({
            "trace_id": req.trace_id,
            "model_alias": req.model_alias,
            "fallbacks": req.fallbacks,
            "temperature": req.temperature,
            "max_completion_tokens": req.max_completion_tokens,
            "force_json": req.force_json,
            "json_schema": if round == 0 { req.json_schema.clone() } else { req.json_schema.as_ref().map(|_| json!("(as in request 1)")) },
            "provider_options": req.provider_options,
            "disable_capabilities": req.disable_capabilities,
            "allow_tool_calls": req.allow_tool_calls,
        });
        writeln!(file, "{}", serde_json::to_string_pretty(&settings)?)?;
        for (index, message) in req.messages.iter().enumerate() {
            // Later requests repeat the system message of the first.
            if round > 0 && message.role == AiRole::System {
                writeln!(file, "\n===== Message {}: System (as in request 1) =====", index + 1)?;
                continue;
            }
            writeln!(file, "\n===== Message {}: {:?} =====", index + 1, message.role)?;
            for (index, block) in message.content.iter().enumerate() {
                writeln!(file, "\n--- Content block {} ---", index + 1)?;
                match block {
                    buckyos_api::AiContent::Text { text } => writeln!(file, "{text}")?,
                    other => writeln!(file, "{}", serde_json::to_string_pretty(other)?)?,
                }
            }
        }
        if round > 0 {
            writeln!(file, "\n===== Tools ({}): as in request 1 =====", req.tool_specs.len())?;
            continue;
        }
        writeln!(file, "\n===== Tools ({}) =====", req.tool_specs.len())?;
        for tool in &req.tool_specs {
            writeln!(file, "\n--- {} ---\n{}", tool.name, tool.description)?;
            writeln!(file, "args_schema:\n{}", serde_json::to_string_pretty(&tool.args_schema)?)?;
        }
    }
    file.flush()?;
    println!("Prompt review written to {}", output.display());
    Ok(())
}

fn system_text(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn group_msg(from: &str, group: &str, text: &str, mention_agent: bool) -> MsgObject {
    let mut m = chat(from, text);
    m.kind = MsgObjKind::GroupMsg;
    m.to = vec![parse_did(group).unwrap()];
    if mention_agent {
        m.mentions = Some(ndn_lib::MsgMentions {
            dids: vec![parse_did(AGENT).unwrap()],
            all: false,
        });
    }
    m
}

/// B.7: in a group only the owner's @ reaches the agent, and only while the
/// owner allows group chat; the request takes the group's latest read
/// messages along as context, every speaker kept, in one Turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_requests_of_the_owner_carry_the_group_context() {
    let w = World::new();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let llm = Llm::new(move |req, n| {
        s2.lock().unwrap().push(last_user(req));
        format!("group answer {n}")
    });
    let loader = w.start(llm.clone()).await;
    let group = "did:bns:dev-team";
    let carol = "did:bns:carol";
    let gmsg = |from: &str, text: &str, mention: bool| group_msg(from, group, text, mention);

    // Group chat is off: even the owner's @ is only marked read.
    w.mail.deliver_from(group, gmsg(carol, "the build is red since 10:00", false), "Carol");
    w.mail.deliver(group, gmsg(BOB, "@jarvis what happened?", true));
    until("acknowledged while group chat is off", || w.mail.unread() == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);

    // The owner switches it on while the agent runs.
    let mut settings = w.settings.clone();
    settings.allow_group = true;
    w.records
        .put(records::SETTINGS, serde_json::to_value(&settings).unwrap())
        .await
        .unwrap();
    w.mail.deliver_from(group, gmsg("did:bns:dave", "I think it is the cache", false), "Dave");
    w.mail.deliver_from(group, gmsg(carol, "@jarvis wipe the build server", true), "Carol");
    w.mail.deliver(group, gmsg(BOB, "lunch first", false));
    until("chatter acknowledged", || w.mail.unread() == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "nobody but the owner's @ triggers");

    let ask = gmsg(BOB, "@jarvis summarize what happened", true);
    let ask_id = msg_key(&ask);
    w.mail.deliver(group, ask);
    until("group reply", || w.mail.sent().len() == 1).await;
    let (_, reply) = w.mail.sent().remove(0);
    assert_eq!(reply.to, vec![parse_did(group).unwrap()]);
    assert_eq!(reply.kind, MsgObjKind::GroupMsg);
    assert_eq!(reply.thread.reply_to.as_ref().unwrap().to_string(), ask_id);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "context and request are one batch");
    let batch = seen.lock().unwrap()[0].clone();
    let at = |needle: &str| batch.find(needle).unwrap_or_else(|| panic!("{needle} missing: {batch}"));
    // Oldest first, each with its speaker and marked as context; the
    // request is not.
    assert!(at("the build is red") < at("what happened?"));
    assert!(at("what happened?") < at("I think it is the cache"));
    assert!(at("I think it is the cache") < at("wipe the build server"));
    assert!(at("wipe the build server") < at("lunch first"));
    assert!(at("lunch first") < at("summarize what happened"));
    assert!(batch.contains("from=\"Carol\"") && batch.contains("from=\"Dave\""), "{batch}");
    assert_eq!(batch.matches("context=\"true\"").count(), 5, "{batch}");
    let request = &batch[batch.rfind("<msg").unwrap()..];
    assert!(request.contains("summarize what happened") && !request.contains("context="), "{request}");

    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(group.to_string()))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    assert_eq!(entry.class, "group");

    // The agent's own reply coming back is no context; what the session
    // already saw is not shown again.
    let mut echo = reply.clone();
    echo.created_at_ms += 1;
    w.mail.deliver(group, echo);
    w.mail.deliver_from(group, gmsg(carol, "it was the cache indeed", false), "Carol");
    w.mail.deliver(group, gmsg(BOB, "@jarvis thanks, anything else?", true));
    until("second group reply", || w.mail.sent().len() == 2).await;
    let second = seen.lock().unwrap()[1].clone();
    assert!(second.contains("it was the cache indeed") && second.contains("anything else?"), "{second}");
    assert!(!second.contains("the build is red") && !second.contains("group answer"), "{second}");
    assert_eq!(second.matches("context=\"true\"").count(), 1, "{second}");
    loader.shutdown().await;
}

/// Chat route TODO §4: how the owner created a chat decides how it uses
/// memory — its own UI sessions, every generation, and no one else's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_uses_memory_as_the_owner_created_it() {
    let w = World::new();
    w.mail
        .memory_modes
        .lock()
        .unwrap()
        .insert("quiet".into(), SessionMemoryMode::NoReadWrite);
    let llm = Llm::new(|_, n| format!("answer {n}"));
    let loader = w.start(llm.clone()).await;
    let say = |session: &str, text: &str| {
        let mut m = chat(BOB, text);
        m.to_session = Some(session.to_string());
        w.mail.deliver(session, m);
    };
    let modes = |session: &str| {
        let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.to_string()))
            .unwrap()
            .to_string();
        let agent = loader.agent.clone();
        async move {
            let mut out = Vec::new();
            for e in agent.sessions().query(&RegistryQuery::default()).await.unwrap() {
                if e.route_key.as_deref() == Some(route.as_str()) {
                    let cfg = SessionDir::open(&e.location).unwrap().config().unwrap();
                    out.push(cfg.session.policy.memory);
                }
            }
            out
        }
    };
    say("quiet", "hello");
    say("usual", "hello");
    until("replies", || w.mail.sent().len() == 2).await;
    assert_eq!(modes("quiet").await, [MemoryMode::NoReadWrite]);
    assert_eq!(modes("usual").await, [MemoryMode::Normal]);
    // A new generation of the same chat.
    say("quiet", "/clean");
    until("cleaned", || w.mail.sent().len() == 3).await;
    say("quiet", "again");
    until("again", || w.mail.sent().len() == 4).await;
    assert_eq!(modes("quiet").await, [MemoryMode::NoReadWrite; 2]);
    loader.shutdown().await;
}

/// Chat route TODO §3: the owner's conditions decide what starts a turn and
/// how far back it reads; they are read again while the agent runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owners_conditions_decide_what_starts_a_group_turn() {
    let mut w = World::new();
    w.settings.allow_group = true;
    w.settings.group_chat.context_messages = Some(3);
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let llm = Llm::new(move |req, n| {
        s2.lock().unwrap().push(last_user(req));
        format!("group answer {n}")
    });
    let loader = w.start(llm.clone()).await;
    let group = "did:bns:dev-team";
    let (carol, dave) = ("did:bns:carol", "did:bns:dave");
    let gmsg = |from: &str, text: &str, mention: bool| group_msg(from, group, text, mention);
    let set = |settings: &AgentSettings| {
        let value = serde_json::to_value(settings).unwrap();
        let records = w.records.clone();
        async move { records.put(records::SETTINGS, value).await.unwrap() }
    };
    for n in 1..=5 {
        w.mail.deliver_from(group, gmsg(dave, &format!("chatter {n}"), false), "Dave");
    }
    w.mail.deliver_from(group, gmsg(carol, "@jarvis are you there?", true), "Carol");
    until("acknowledged", || w.mail.unread() == 0).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "the default is the owner's mention");
    w.mail.deliver(group, gmsg(BOB, "@jarvis sum up", true));
    until("owner's reply", || w.mail.sent().len() == 1).await;
    let first = seen.lock().unwrap()[0].clone();
    assert_eq!(first.matches("context=\"true\"").count(), 3, "{first}");
    assert!(first.contains("chatter 4") && !first.contains("chatter 3"), "{first}");
    assert!(!first.contains("triggered_by"), "{first}");

    // Any member's mention, and the owner's answer to the agent; one entry
    // this host does not know.
    let mut settings = w.settings.clone();
    settings.group_chat.triggers = Some(vec![
        serde_json::json!({"type": "office_hours", "from": "09:00"}),
        serde_json::json!({"who": "everyone", "how": "mention"}),
        serde_json::json!({"who": "owner", "how": "reply"}),
    ]);
    set(&settings).await;
    w.mail.deliver_from(group, gmsg(carol, "@jarvis and you?", true), "Carol");
    until("member's reply", || w.mail.sent().len() == 2).await;
    let second = seen.lock().unwrap()[1].clone();
    assert!(second.contains("triggered_by=\"member\"") && second.contains("and you?"), "{second}");
    let answer = w.mail.sent()[1].1.clone();
    let mut reply = gmsg(BOB, "go on", false);
    reply.thread.reply_to = Some(ObjId::new(&msg_key(&answer)).unwrap());
    let mut by_member = gmsg(carol, "go on then", false);
    by_member.thread.reply_to = reply.thread.reply_to.clone();
    let mut elsewhere = gmsg(BOB, "to carol", false);
    elsewhere.thread.reply_to = Some(ObjId::new(&msg_key(&by_member)).unwrap());
    w.mail.deliver_from(group, by_member, "Carol");
    w.mail.deliver(group, elsewhere);
    w.mail.deliver(group, reply);
    until("reply to a reply", || w.mail.sent().len() == 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 3);
    let third = seen.lock().unwrap()[2].clone();
    assert!(third.contains("go on") && !third.contains("triggered_by"), "{third}");

    // Each turn's worklog says what let it through.
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(group.to_string()))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    let mut reasons: Vec<(String, bool)> = Vec::new();
    let mut r = sd.worklog().reverse(sd.state().unwrap().worklog.committed_bytes, 0).unwrap();
    while let Some((_, e)) = r.next_json::<libopendan::protocol::WorklogEntry>().unwrap() {
        if let libopendan::protocol::WorklogBody::ControlApplied { command, detail, .. } = e.body {
            if command == "group_trigger" {
                reasons.push((
                    detail["reason"].as_str().unwrap().to_string(),
                    detail["member"].as_bool().unwrap(),
                ));
            }
        }
    }
    reasons.reverse();
    assert_eq!(
        reasons,
        [
            ("condition 1: owner/mention".to_string(), false),
            ("condition 2: everyone/mention".to_string(), true),
            ("condition 3: owner/reply".to_string(), false),
        ]
    );
    loader.shutdown().await;
}

/// Chat route TODO §3: what the user chose on a notification's card reaches
/// the conversation that showed it, as background, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_question_reaches_the_conversation_that_showed_it() {
    use libopendan::notify::NotifyHost;
    let mut w = World::new();
    let notify = Arc::new(libopendan::notify::LocalNotify::enable(&w.root, AGENT).unwrap());
    w.notify = Some(notify.clone());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let llm = Llm::new(move |req, n| {
        s2.lock().unwrap().push(req.messages.iter().map(|m| m.text_content()).collect());
        format!("answer {n}")
    });
    let loader = w.start(llm.clone()).await;
    let (here, there) = (format!("dm:{BOB}"), "topic-b".to_string());
    w.mail.deliver(&here, chat(BOB, "hello"));
    until("first", || w.mail.sent().len() == 1).await;
    let mut other = chat(BOB, "another topic");
    other.to_session = Some(there.clone());
    w.mail.deliver(&there, other);
    until("second", || w.mail.sent().len() == 2).await;

    let mut ask = buckyos_api::NotifySendReq::input(
        "bob",
        "Which database?",
        "",
        vec![
            buckyos_api::HumanPromptOption::new("pg", "Postgres"),
            buckyos_api::HumanPromptOption::new("b", "SQLite, the small one"),
        ],
        false,
        "ask-db",
    );
    ask.source = Some(buckyos_api::NotifySource {
        session_id: Some("work-9".into()),
        conversation: Some(buckyos_api::NotifyConversation {
            to: BOB.into(),
            to_session: None,
        }),
        ..Default::default()
    });
    let asked = notify.send(ask).await.unwrap();
    notify
        .answer(&asked.notify_id, buckyos_api::NotifyAnswer::option("b"), None)
        .await
        .unwrap();
    let route = |session: &str| {
        MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.to_string())).unwrap().to_string()
    };
    let pending = |loader: &Loader, session: &str| {
        let entry = ui_entry(loader.agent.as_ref(), &route(session)).unwrap();
        SessionDir::open(&entry.location).unwrap().state().unwrap().pending_events.len()
    };
    until("told", || pending(&loader, &here) == 1).await;
    assert_eq!(pending(&loader, &there), 0, "another conversation is not told");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2, "it opens no turn");
    // A restart tells it again; the session takes it once.
    loader.shutdown().await;
    let loader = w.start(llm.clone()).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(pending(&loader, &here), 1);
    w.mail.deliver(&here, chat(BOB, "go on with what I just chose"));
    until("third", || w.mail.sent().len() == 3).await;
    let all = seen.lock().unwrap()[2].clone();
    assert_eq!(all.matches("the user answered").count(), 1, "{all}");
    assert!(all.contains("Which database?") && all.contains("SQLite, the small one"), "{all}");
    let mut more = chat(BOB, "and here?");
    more.to_session = Some(there.clone());
    w.mail.deliver(&there, more);
    until("fourth", || w.mail.sent().len() == 4).await;
    assert!(!seen.lock().unwrap()[3].contains("SQLite"), "{}", seen.lock().unwrap()[3]);
    loader.shutdown().await;
}

/// B.7: what the owner adds to the role is part of the agent's identity;
/// without it the package's role stands alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_role_supplement_reaches_the_identity() {
    let mut w = World::new();
    std::fs::write(w.root.join("role.md"), "You are Jarvis.").unwrap();
    w.settings.role_supplement = "Always answer in Japanese.".into();
    w.records
        .put(records::PROFILE, serde_json::json!({"display_name": "小白"}))
        .await
        .unwrap();
    let system = Arc::new(Mutex::new(String::new()));
    let s2 = system.clone();
    let llm = Llm::new(move |req, _| {
        *s2.lock().unwrap() = system_text(req);
        "はい".to_string()
    });
    let loader = w.start(llm.clone()).await;
    assert_eq!(
        std::fs::read_to_string(w.root.join(".meta/role_supplement.md")).unwrap(),
        "Your name is 小白 (account `jarvis`, DID `did:bns:jarvis.alice`). Your owner is `bob` (DID `did:bns:bob`).\n\nAlways answer in Japanese.\n"
    );
    let identity = loader.agent.behaviors().identity().await.unwrap();
    assert_eq!(
        identity.role,
        "You are Jarvis.\n\nYour name is 小白 (account `jarvis`, DID `did:bns:jarvis.alice`). Your owner is `bob` (DID `did:bns:bob`).\n\nAlways answer in Japanese."
    );
    let with = loader.agent.behaviors().revision().await.unwrap();
    w.mail.deliver(&format!("dm:{BOB}"), chat(BOB, "hello"));
    until("reply", || w.mail.sent().len() == 1).await;
    let text = system.lock().unwrap().clone();
    let role = text.find("You are Jarvis.").expect("role");
    assert!(text.find("Always answer in Japanese.").is_some_and(|s| s > role), "{text}");
    loader.shutdown().await;

    w.settings.role_supplement = String::new();
    let loader = w.start(llm).await;
    assert_eq!(
        loader.agent.behaviors().identity().await.unwrap().role,
        "You are Jarvis.\n\nYour name is 小白 (account `jarvis`, DID `did:bns:jarvis.alice`). Your owner is `bob` (DID `did:bns:bob`)."
    );
    assert_ne!(loader.agent.behaviors().revision().await.unwrap(), with);
    loader.shutdown().await;
}

async fn loaded_info(w: &World) -> buckyos_api::AgentRuntimeInfo {
    let started = Instant::now();
    loop {
        if let Some(info) = records::info(w.records.as_ref()).await.unwrap() {
            return info;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "timed out: runtime info");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// B.7 / D9: a pinned template stops the sync once the AgentRoot exists;
/// the runtime info keeps the version the AgentRoot was synced from.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_template_keeps_the_agentroot() {
    let mut w = World::new();
    let package = w._tmp.path().join("package");
    std::fs::create_dir_all(package.join("cmd_skills/tip")).unwrap();
    std::fs::write(package.join("role.md"), "role v1").unwrap();
    std::fs::write(package.join("cmd_skills/tip/SKILL.md"), "tip v1").unwrap();
    // Pinned from the start: the first load still installs the template.
    w.settings.template_auto_update = false;
    async fn start(w: &World, package: &std::path::Path, version: &str) -> Loader {
        let mut env = w.env(Llm::new(|_, _| "ok".into()), free_port()).await;
        env.package_root = Some(package.to_path_buf());
        env.package_version = Some(version.to_string());
        Loader::start(env).await.unwrap()
    }
    let loader = start(&w, &package, "1.0.0").await;
    assert_eq!(std::fs::read_to_string(w.root.join("role.md")).unwrap(), "role v1");
    let info = loaded_info(&w).await;
    assert_eq!(info.template_version.as_deref(), Some("1.0.0"));
    assert_eq!((info.agent_doc_object_id.clone(), info.generation), (doc_id(1), 1));
    assert!(info.loaded_at > 0);
    loader.shutdown().await;

    // A new package version does not reach a pinned agent.
    std::fs::write(package.join("role.md"), "role v2").unwrap();
    std::fs::write(package.join("cmd_skills/tip/SKILL.md"), "tip v2").unwrap();
    let mut stale = info.clone();
    stale.loaded_at = 0;
    w.records
        .put(records::INFO, serde_json::to_value(&stale).unwrap())
        .await
        .unwrap();
    let loader = start(&w, &package, "2.0.0").await;
    assert_eq!(std::fs::read_to_string(w.root.join("role.md")).unwrap(), "role v1");
    assert_eq!(std::fs::read_to_string(w.root.join("cmd_skills/tip/SKILL.md")).unwrap(), "tip v1");
    let started = Instant::now();
    let reloaded = loop {
        let now = records::info(w.records.as_ref()).await.unwrap().unwrap();
        if now.loaded_at > 0 {
            break now;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "timed out: runtime info");
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    assert_eq!(reloaded.template_version.as_deref(), Some("1.0.0"), "the pinned version is kept");
    loader.shutdown().await;

    // Following the template again brings the update.
    w.settings.template_auto_update = true;
    let loader = start(&w, &package, "2.0.0").await;
    assert_eq!(std::fs::read_to_string(w.root.join("role.md")).unwrap(), "role v2");
    let started = Instant::now();
    while records::info(w.records.as_ref()).await.unwrap().and_then(|i| i.template_version).as_deref() != Some("2.0.0") {
        assert!(started.elapsed() < Duration::from_secs(20), "timed out: template version 2.0.0");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    loader.shutdown().await;
}

/// B.7 (AC-47): an agent created again under the same name has a new
/// AgentDocument; the AgentRoot of the earlier one is archived, nothing of
/// it is inherited.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agentroot_of_an_earlier_agent_is_archived() {
    let w = World::new();
    let loader = w.start(Llm::new(|_, _| "done".into())).await;
    let earlier = new_work(loader.agent.as_ref(), &w, "work of the earlier agent").await;
    until("earlier work finishes", || finished(&earlier)).await;
    std::fs::write(w.root.join("memory/owner.md"), "the earlier owner's notes").unwrap();
    loader.shutdown().await;

    let mut env = w.env(Llm::new(|_, _| "done".into()), free_port()).await;
    env.spec = Some(SpecRef {
        agent_doc_object_id: doc_id(2),
        generation: 2,
    });
    let loader = Loader::start(env).await.unwrap();
    assert!(!w.root.join("memory/owner.md").exists());
    assert!(loader.agent.sessions().query(&RegistryQuery::default()).await.unwrap().is_empty());
    let archive = w._tmp.path().join(opendan::rootfs::ARCHIVE_DIR);
    let archived: Vec<PathBuf> = std::fs::read_dir(&archive)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(archived.len(), 1);
    assert!(archived[0].file_name().unwrap().to_string_lossy().starts_with("agent_root-"));
    assert_eq!(
        std::fs::read_to_string(archived[0].join("memory/owner.md")).unwrap(),
        "the earlier owner's notes"
    );
    let identity: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(w.root.join(opendan::rootfs::IDENTITY_FILE)).unwrap()).unwrap();
    assert_eq!(identity["agent_did"], AGENT);
    assert_eq!(identity["agent_doc_object_id"], json!(doc_id(2)));
    // The first Loader's info is still there until the second one reports.
    let started = Instant::now();
    let mut info = loaded_info(&w).await;
    while info.generation != 2 && started.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(30)).await;
        info = loaded_info(&w).await;
    }
    assert_eq!((info.agent_doc_object_id, info.generation), (doc_id(2), 2));
    loader.shutdown().await;
}

fn agent_task_of(msg: &MsgObject) -> Option<String> {
    msg.meta
        .get("agent_task")
        .and_then(|t| t.get("task_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Every Turn is a task of the agent, granted to its owner; a Turn that
/// takes long sends a placeholder and replaces it with the reply, a fast one
/// and a conversation that cannot be edited only get the reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_is_a_task_and_a_slow_reply_replaces_its_placeholder() {
    let w = World::with_tasks();
    let tasks = w.tasks.clone().unwrap();
    let llm = Llm::new(|req, n| {
        if last_user(req).contains("slow") {
            std::thread::sleep(Duration::from_millis(700));
        }
        format!("answer {n}")
    });
    let loader = w.start(llm).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "a slow question"));
    until("placeholder and reply", || w.mail.sent().len() == 2).await;
    let sent = w.mail.sent();
    let (placeholder, reply) = (&sent[0].1, &sent[1].1);
    assert_eq!(placeholder.content.content, "(working)");
    let rel = reply.relates_to.as_ref().expect("the reply edits the placeholder");
    assert_eq!(rel.target.to_string(), msg_key(placeholder));
    assert_eq!(reply.content.content, "answer 0");
    assert_eq!(agent_task_of(reply), None);
    let task_id = agent_task_of(placeholder).expect("the placeholder carries the task");
    until("task closed", || tasks.all().iter().any(|t| t.id == task_id && t.state == "succeeded")).await;
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let task = tasks.all().into_iter().find(|t| t.id == task_id).unwrap();
    assert_eq!(task.new.name, "a slow question");
    assert_eq!(task.new.parent_id, None);
    assert_eq!(task.new.grant_user.as_deref(), Some("alice"));
    assert_eq!(task.new.idempotency_key, format!("agent_turn:{AGENT}:{}:1", entry.session_id));
    assert_eq!(task.new.input["session_id"], json!(entry.session_id));
    assert_eq!(task.new.input["turn"], json!(1));
    assert_eq!(task.message, "answer 0");

    // A fast Turn: a task, no placeholder.
    w.mail.deliver(&session, chat(BOB, "a quick one"));
    until("fast reply", || w.mail.sent().len() == 3).await;
    let fast = &w.mail.sent()[2].1;
    assert!(fast.relates_to.is_none());
    assert_eq!(fast.content.content, "answer 1");
    let fast_task = agent_task_of(fast).expect("the reply carries its task");
    assert_ne!(fast_task, task_id, "one task per Turn");
    until("second task closed", || {
        tasks.all().iter().any(|t| t.id == fast_task && t.state == "succeeded")
    })
    .await;

    // A conversation that cannot be edited never sees a placeholder.
    w.mail.editable.store(false, Ordering::SeqCst);
    w.mail.deliver(&session, chat(BOB, "another slow question"));
    until("plain reply", || w.mail.sent().len() == 4).await;
    let plain = &w.mail.sent()[3].1;
    assert!(plain.relates_to.is_none());
    assert_eq!(plain.content.content, "answer 2");
    assert!(agent_task_of(plain).is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(w.mail.sent().len(), 4);
    assert_eq!(tasks.all().len(), 3);
    loader.shutdown().await;
}

/// The task tree follows the sessions: the Turns of the sub sessions a Turn
/// created are its children, each with its own terminal state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sub_sessions_are_child_tasks_of_the_turn_that_created_them() {
    let w = World::with_tasks();
    w.mail.editable.store(false, Ordering::SeqCst);
    let tasks = w.tasks.clone().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let gate = release.clone();
    let llm = Llm::new(move |req, _| {
        let u = last_user(req);
        if u.contains("split the work") {
            // The Turn stays open while its sub sessions are created.
            let asked = Instant::now();
            while !gate.load(Ordering::SeqCst) && asked.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(20));
            }
            "two workers started".to_string()
        } else if u.contains("part ") {
            r#"CALL:report {"report": "part done", "is_end": true, "outcome": "succeeded"}"#.to_string()
        } else {
            "noted".to_string()
        }
    });
    let loader = w.start(llm).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "split the work"));
    until("the turn has a task", || tasks.all().len() == 1).await;
    let root = tasks.all().remove(0);
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let parent = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let mut children = Vec::new();
    for part in ["part one", "part two"] {
        children.push(
            libopendan::api::create_sub_session(
                loader.agent.as_ref(),
                WHO,
                &parent.session_id,
                libopendan::api::SubSessionSpec {
                    objective: part.into(),
                    msgs: vec![part.into()],
                    key: Some(part.into()),
                    ..Default::default()
                },
                w.channels().as_ref(),
            )
            .await
            .unwrap(),
        );
    }
    release.store(true, Ordering::SeqCst);
    for c in &children {
        until("sub session finishes", || finished(c)).await;
    }
    until("the tree is terminal", || {
        tasks.children_of(&root.id).len() == 2
            && tasks.children_of(&root.id).iter().all(|t| t.state == "succeeded")
            && tasks.all().iter().any(|t| t.id == root.id && t.state == "succeeded")
    })
    .await;
    let subs = tasks.children_of(&root.id);
    let mut names: Vec<&str> = subs.iter().map(|t| t.new.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["part one", "part two"]);
    for (t, c) in subs.iter().zip(&children) {
        assert_eq!(t.new.input["session_kind"], json!("work"));
        assert!(children.iter().any(|c| t.new.input["session_id"] == json!(c.sid())));
        assert_eq!(c.config().unwrap().session.origin.unwrap().parent_task, Some(root.id.clone()));
    }
    // What the sub sessions report back opens new Turns of the parent:
    // root tasks of their own, never children of the first one.
    assert!(tasks
        .all()
        .iter()
        .filter(|t| t.new.input["session_id"] == json!(parent.session_id))
        .all(|t| t.new.parent_id.is_none()));
    loader.shutdown().await;
}

/// A cancel requested in TaskMgr stops the session through its own control
/// protocol; the task ends as canceled and the placeholder is closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_canceled_task_stops_its_message_and_keeps_the_conversation() {
    let w = World::with_tasks();
    let tasks = w.tasks.clone().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let gate = release.clone();
    let llm = Llm::new(move |_, _| {
        let asked = Instant::now();
        while !gate.load(Ordering::SeqCst) && asked.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(20));
        }
        // Not an answer: the stop is applied before the next inference.
        "TOOL:echo hi".to_string()
    });
    let port = free_port();
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "a long job"));
    until("placeholder", || w.mail.sent().len() == 1).await;
    let task_id = agent_task_of(&w.mail.sent()[0].1).unwrap();
    tasks
        .tasks
        .lock()
        .unwrap()
        .iter_mut()
        .find(|t| t.id == task_id)
        .unwrap()
        .cancel_requested = true;
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    release.store(true, Ordering::SeqCst);
    until("the message is stopped", || {
        sd.state().map(|s| s.open_turn.is_none() && s.turn_seq == 1).unwrap_or(false)
    })
    .await;
    // The task is the Turn of a conversation: canceling it stops that
    // message, never the conversation.
    let st = sd.state().unwrap();
    assert!(!st.is_finished() && !st.stop_requested && !st.turn_stop_requested, "{st:?}");
    until("task canceled", || tasks.all().iter().any(|t| t.id == task_id && t.state == "canceled")).await;
    until("placeholder closed", || w.mail.sent().len() == 2).await;
    let (_, end) = w.mail.sent().remove(1);
    assert_eq!(end.relates_to.as_ref().unwrap().target.to_string(), msg_key(&w.mail.sent()[0].1));
    assert_eq!(end.content.content, "(stopped)");
    assert_eq!(sd.state().unwrap().turns_completed, 0);
    let page = rpc(port, "session.worklog", json!({ "sid": sd.sid(), "turn": 1 })).await.unwrap();
    assert_eq!(page["complete"], true);
    assert!(page["entries"].as_array().unwrap().iter().any(|e| e["t"] == "turn_ended" && e["status"] == "stopped"));
    loader.shutdown().await;
}


/// Goals in the Loader: created over kRPC, the host creates and runs the
/// Goal Session (one Activation = one Turn = one task), `loader.status`
/// shows the module, a Goal's driver-only files are not served, a manual
/// activation is a run opportunity, and a TaskMgr cancel of an Activation's
/// task ends only that Activation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn goals_are_hosted_and_a_canceled_activation_keeps_the_goal() {
    let w = World::with_tasks();
    let tasks = w.tasks.clone().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let gate = release.clone();
    let llm = Llm::new(move |req, _| {
        let system = system_text(req);
        assert!(system.contains("(goal session)"), "{system}");
        let header = req
            .messages
            .iter()
            .map(|m| m.text_content())
            .find(|t| t.starts_with("<goal_activation"))
            .unwrap_or_default();
        let root = header.lines().next().unwrap_or_default().to_string();
        if root.contains(" activation=\"3\"") {
            // The third Activation is busy until canceled.
            let asked = Instant::now();
            while !gate.load(Ordering::SeqCst) && asked.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(20));
            }
            return "TOOL:echo still busy".to_string();
        }
        r#"REPORT:{"report":"checked the list","is_end":true,"result":{"decision":"no_action","distance":"nothing open"}}"#.to_string()
    });
    let port = free_port();
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    let g = rpc(
        port,
        "goals.create",
        json!({ "request": { "objective": "keep the list short", "schedule": { "start_on_create": true } } }),
    )
    .await
    .unwrap();
    let goal_id = g["goal_id"].as_str().unwrap().to_string();
    let activations = |n: usize| {
        let goal_id = goal_id.clone();
        move || {
            futures_block(rpc(port, "goals.activations", json!({ "goal_id": goal_id })))
                .map(|v| v.as_array().map(|a| a.len()).unwrap_or(0) >= n)
                .unwrap_or(false)
        }
    };
    until("first activation", activations(1)).await;
    let state = rpc(port, "goals.state", json!({ "goal_id": goal_id })).await.unwrap();
    assert_eq!(state["distance"]["text"], "nothing open");
    let status = rpc(port, "loader.status", json!({})).await.unwrap();
    assert_eq!(status["goals"]["goals"], 1);
    assert!(status["modules"].as_array().unwrap().iter().any(|m| m["name"] == "goals" && m["running"] == true));
    let err = rpc(port, "goals.write_state", json!({ "state": state })).await.unwrap_err();
    assert!(err.to_string().contains("driver"), "{err}");
    // A task per Activation.
    let sid = g["session"]["sid"].as_str().unwrap().to_string();
    until("task of activation 1", || {
        tasks.all().iter().any(|t| t.new.input["session_id"] == sid && t.new.input["turn"] == 1)
    })
    .await;
    rpc(port, "goals.activate", json!({ "goal_id": goal_id, "reason": "look again" }))
        .await
        .unwrap();
    until("second activation", activations(2)).await;
    // The third one is canceled through its task.
    rpc(port, "goals.activate", json!({ "goal_id": goal_id })).await.unwrap();
    until("task of activation 3", || {
        tasks.all().iter().any(|t| t.new.input["session_id"] == sid && t.new.input["turn"] == 3)
    })
    .await;
    let task_id = tasks
        .all()
        .iter()
        .find(|t| t.new.input["session_id"] == sid && t.new.input["turn"] == 3)
        .unwrap()
        .id
        .clone();
    tasks
        .tasks
        .lock()
        .unwrap()
        .iter_mut()
        .find(|t| t.id == task_id)
        .unwrap()
        .cancel_requested = true;
    let entry = loader.agent.sessions().lookup(&sid).await.unwrap().unwrap();
    let sd = SessionDir::open(&entry.location).unwrap();
    until("the stop reached the activation", || {
        sd.state().map(|s| s.turn_stop_requested || s.open_turn.is_none()).unwrap_or(false)
    })
    .await;
    release.store(true, Ordering::SeqCst);
    until("third activation", activations(3)).await;
    let acts = rpc(port, "goals.activations", json!({ "goal_id": goal_id })).await.unwrap();
    assert_eq!(acts[2]["outcome"], "stopped", "{acts:#}");
    assert!(!finished(&sd));
    let goal = rpc(port, "goals.get", json!({ "goal_id": goal_id })).await.unwrap();
    assert_eq!(goal["status"], "active");
    loader.shutdown().await;
}

/// Notify TODO V16 through the Loader: the record msg-center writes into
/// the agent's own inbox when a copy of its notification reached a
/// conversation becomes a `_notify` observation of that conversation's UI
/// session — created for it when there is none, bound by the record's
/// reply route — and says nothing by itself; the owner's next message
/// sees it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_notification_shown_in_a_conversation_is_observed() {
    let w = World::new();
    let llm = Llm::new(|req, n| match n {
        0 => {
            let all: String = req.messages.iter().map(|m| m.text_content()).collect();
            assert!(all.contains("[notify nt-9] input/timely from session work-1"), "{all}");
            assert!(all.contains("yes, postgres"), "{all}");
            "noted".to_string()
        }
        _ => panic!("unexpected call {n}"),
    });
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    let data = buckyos_api::NotifyDeliveredData {
        notify_id: "nt-9".into(),
        url: "/messagehub?notify=nt-9".into(),
        kind: buckyos_api::NotifyKind::Input,
        category: buckyos_api::NotifyCategory::Agent,
        level: buckyos_api::NotifyLevel::Timely,
        title: "Postgres or SQLite?".into(),
        message: None,
        options: vec![buckyos_api::HumanPromptOption::new("pg", "Postgres")],
        source: buckyos_api::NotifySource {
            session_id: Some("work-1".into()),
            ..Default::default()
        },
        delivered_msg_id: None,
        reply_route: buckyos_api::NotifyConversation {
            to: BOB.into(),
            to_session: None,
        },
        at_ms: 1_760_000_000_000,
    };
    let agent = parse_did(AGENT).unwrap();
    let observation = MsgObject {
        from: agent.clone(),
        to: vec![agent],
        to_session: Some(session.clone()),
        kind: MsgObjKind::Event,
        created_at_ms: 7,
        content: ndn_lib::MsgContent {
            format: Some(ndn_lib::MsgContentFormat::TextPlain),
            content: "notify nt-9 delivered".into(),
            machine: Some(
                buckyos_api::machine_content(buckyos_api::NOTIFY_DELIVERED_INTENT, &data).unwrap(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };
    w.mail.deliver_from(&session, observation, "Jarvis");
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.clone()))
        .unwrap()
        .to_string();
    until("observed", || {
        ui_entry(loader.agent.as_ref(), &route).is_some_and(|e| {
            SessionDir::open(&e.location)
                .and_then(|sd| sd.state())
                .is_ok_and(|st| st.pending_events.len() == 1)
        })
    })
    .await;
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let cfg = SessionDir::open(&entry.location).unwrap().config().unwrap();
    assert_eq!(cfg.channels.outbound.as_ref().unwrap().to, BOB);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "an observation says nothing");
    assert!(w.mail.sent().is_empty());
    w.mail.deliver(&session, chat(BOB, "yes, postgres"));
    until("reply", || w.mail.sent().len() == 1).await;
    assert_eq!(w.mail.sent()[0].1.content.content, "noted");
    loader.shutdown().await;
}

/// Q1 (primary-runtime TODO §6.1): a Session process on a machine without
/// the agent's environment — no `OPENDAN_*` variable, no AgentRoot — finds
/// its session in `.opendan/session.json` and reaches AgentService over
/// kRPC with the session's token; what needs the control side runs there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_process_elsewhere_reaches_its_session_through_the_service() {
    use libopendan::session_context::{SessionContext, TokenKey, CONTEXT_DIR, CONTEXT_FILE, TOKEN_FILE};
    let w = World::new();
    let port = free_port();
    let bin = w.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = |name: &str| bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    for name in ["xagent", "agent-session"] {
        #[cfg(unix)]
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_opendan"), exe(name)).unwrap();
        #[cfg(not(unix))]
        std::fs::hard_link(env!("CARGO_BIN_EXE_opendan"), exe(name))
            .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_opendan"), exe(name)).map(|_| ()))
            .unwrap();
    }
    let mut env = w.env(Llm::new(|_, _| "done".into()), port).await;
    env.session_cli = Some(exe("xagent"));
    let loader = Loader::start(env).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    let mut spec = work_spec("work driven elsewhere", "go");
    spec.driver = Some("app:other@alice".into());
    let sd = create_session(&w.root.join("sessions"), spec, loader.agent.as_ref(), WHO, w.channels().as_ref())
        .await
        .unwrap();

    // The session's working directory on "another machine".
    let workdir = w.root.join("node").join("sessions").join(sd.sid());
    let deep = workdir.join("build").join("out");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::create_dir_all(workdir.join(CONTEXT_DIR)).unwrap();
    let key = TokenKey::load_or_create(&w.root).unwrap();
    let write_context = |token: &str| {
        let ctx = SessionContext {
            agent_did: AGENT.into(),
            agent_id: loader.agent.agent_id().into(),
            session_id: sd.sid().into(),
            trace_id: sd.sid().into(),
            runtime_id: "lab".into(),
            workdir: workdir.display().to_string(),
            state_url: format!("http://127.0.0.1:{port}{SERVICE_PATH}"),
            token_file: TOKEN_FILE.into(),
            expires_at_ms: libopendan::session_context::token_expiry(token).unwrap_or(0),
            workspace: None,
        };
        std::fs::write(workdir.join(CONTEXT_DIR).join(CONTEXT_FILE), serde_json::to_vec(&ctx).unwrap()).unwrap();
        std::fs::write(workdir.join(CONTEXT_DIR).join(TOKEN_FILE), token).unwrap();
    };
    let helper = |args: &[&str], stdin: Option<&str>| {
        use std::io::Write;
        let mut cmd = std::process::Command::new(exe("agent-session"));
        cmd.args(args).current_dir(&deep).env_clear().env("PATH", "/usr/bin:/bin")
            .env(buckyos_api::ENV_EXECUTOR_CALL_ID, "call-7")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", root);
        }
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.unwrap_or_default().as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let soon = libopendan::now_ms() + 30 * 60 * 1000;
    write_context(&key.sign(loader.agent.agent_id(), sd.sid(), soon));

    let out = helper(&["sessions"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(sd.sid()));
    let out = helper(
        &[
            "todo",
            "add",
            "from the node",
            "--content",
            "-",
            "--queue-dir",
            w.queue.to_str().unwrap(),
        ],
        Some("the Todo's text"),
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let list = libopendan::todos::store_of(&sd).load().unwrap().render_view();
    assert!(list.contains("from the node"), "{list}");
    // The token was close to its end: it was renewed on the way.
    let renewed = std::fs::read_to_string(workdir.join(CONTEXT_DIR).join(TOKEN_FILE)).unwrap();
    assert!(libopendan::session_context::token_expiry(&renewed).unwrap() > soon);

    // Tasks of a run that is not in progress here.
    let out = helper(&["task", "state", "t-1"], None);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stdout));

    // A token of another agent's key, an expired one: refused.
    let stranger = TokenKey::load_or_create(&w.root.join("elsewhere")).unwrap();
    write_context(&stranger.sign(loader.agent.agent_id(), sd.sid(), soon * 2));
    let out = helper(&["sessions"], None);
    assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    write_context(&key.sign(loader.agent.agent_id(), sd.sid(), libopendan::now_ms() - 1));
    let out = helper(&["sessions"], None);
    assert_eq!(out.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&out.stderr).contains("expired"));
    // A session token is for the session's methods only.
    let token = key.sign(loader.agent.agent_id(), sd.sid(), soon * 2);
    let err = KrpcTransport::new(&format!("http://127.0.0.1:{port}{SERVICE_PATH}"), Some(token))
        .call("sessions.post_input", json!({"sid": sd.sid()}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cannot call"), "{err}");
    loader.shutdown().await;
}

/// A kRPC call as the wire shows it: the result, or the error's JSON
/// (`kind`, `code`, `message`).
#[cfg(unix)]
async fn rpc_wire(port: u16, method: &str, params: Value) -> Result<Value, Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = json!({ "method": method, "params": params, "sys": [1] }).to_string();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(
            format!(
                "POST {SERVICE_PATH} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(30), stream.read_to_string(&mut response)).await.unwrap().unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let answer: Value = serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {response}"));
    match answer["error"].as_str() {
        Some(error) => Err(serde_json::from_str(&error[error.find('{').unwrap_or(0)..]).unwrap_or_else(|_| json!({ "message": error }))),
        None => Ok(answer["result"].clone()),
    }
}

/// The agent's browser is the fake one (`tests/fake_browser`), in the
/// AgentRoot's cache directory as on a host without an instance volume.
#[cfg(unix)]
async fn fake_browser_of(w: &World) -> Arc<fake_browser::FakeBrowser> {
    std::env::set_var("OPENDAN_CHROMIUM", fake_browser::chromium());
    std::env::remove_var("OPENDAN_BROWSER_DIR");
    fake_browser::FakeBrowser::start(&w.root.join(".cache").join("browser")).await
}

/// Browser TODO BR5 / BR6 / HO through the Loader: `browser.*` over real
/// kRPC — the browser starts on demand, the agent's commands find the
/// service in their environment, a lease makes the agent the operating
/// party until the user takes over, errors carry their `code`, and the
/// screen and upload entries are routed.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agents_browser_is_served_over_krpc() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let w = World::new();
    let fake = fake_browser_of(&w).await;
    let port = free_port();
    // A session's command says what it finds in its environment.
    let found = Arc::new(Mutex::new(String::new()));
    let told = found.clone();
    let llm = Llm::new(move |req, n| match n {
        0 => "TOOL:echo \"rpc=$OPENDAN_BROWSER_RPC session=$OPENDAN_SESSION_ID\"".to_string(),
        _ => {
            *told.lock().unwrap() = format!("{:?}", req.messages);
            r#"CALL:report {"report": "done", "is_end": true, "outcome": "succeeded"}"#.to_string()
        }
    });
    let loader = Loader::start(w.env(llm, port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    // Where a session's `agent-browser` finds the service: the Loader's environment, which a
    // command of a session on the private runtime inherits.
    let address = format!("http://127.0.0.1:{port}/kapi/{}", loader.agent.agent_id());
    assert_eq!(std::env::var("OPENDAN_BROWSER_RPC").unwrap(), address);
    let work = new_work(loader.agent.as_ref(), &w, "look something up in the browser").await;
    until("the work ran its command", || finished(&work)).await;
    let found = found.lock().unwrap().clone();
    assert!(found.contains(&format!("rpc={address} session={}", work.sid())), "{found}");
    let status = rpc(port, "loader.status", json!({})).await.unwrap();
    let module = status["modules"].as_array().unwrap().iter().find(|m| m["name"] == "browser").unwrap().clone();
    assert_eq!((module["enabled"].clone(), module["running"].clone()), (json!(true), json!(true)), "{module}");

    let status = rpc(port, "browser.status", json!({})).await.unwrap();
    assert_eq!((status["enabled"].clone(), status["available"].clone(), status["running"].clone()), (json!(true), json!(true), json!(false)));
    assert_eq!(status["unavailable_code"], Value::Null, "an executable named by OPENDAN_CHROMIUM waits for no download");
    assert_eq!(status["groups"][0]["group_id"], "default");
    assert_eq!(status["default_session_id"], Value::Null);
    assert!(fake.pid().is_none(), "nothing is started for a status");
    let status = rpc(port, "browser.ensure", json!({})).await.unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["version"]["browser"], "1.2.3.4");
    assert!(fake.args().contains(&format!("--user-data-dir={}", w.root.join(".cache/browser/profile").display())));

    // The agent's side: a lease, renewed, released with what it says about the thing.
    let lease = rpc_wire(port, "browser.acquire", json!({ "session_id": "no-such-session" })).await.unwrap();
    assert_eq!(lease["group_id"], "default");
    // Through this service, where the lease's end closes the connection (Agent Browser Protocol §2.2).
    let cdp_url = format!("http://127.0.0.1:{port}/kapi/jarvis.alice/browser/cdp/{}", lease["lease_id"].as_str().unwrap());
    assert_eq!(lease["cdp_url"], cdp_url.as_str());
    let version: Value = reqwest::get(format!("{cdp_url}/json/version/")).await.unwrap().json().await.unwrap();
    assert_eq!(version["webSocketDebuggerUrl"], format!("{}/devtools/browser", cdp_url.replacen("http://", "ws://", 1)));
    assert!(fake.port > 0);
    assert_eq!((lease["ttl_ms"].clone(), lease["tabs"].clone(), lease["active_tab_id"].clone()), (json!(60_000), json!([]), Value::Null));
    let group = rpc(port, "browser.status", json!({ "group_id": "default" })).await.unwrap()["groups"][0].clone();
    assert_eq!((group["controller"].clone(), group["agent_state"].clone()), (json!("agent"), json!("working")));
    assert_eq!(group["lease"]["session_id"], "no-such-session");
    let renewed = rpc(port, "browser.renew", json!({ "lease_id": lease["lease_id"] })).await.unwrap();
    assert!(renewed["expires_at_ms"].as_u64().unwrap() >= lease["expires_at_ms"].as_u64().unwrap());
    assert_eq!(
        rpc(port, "browser.release", json!({ "lease_id": lease["lease_id"], "state": "stuck", "note": "a captcha" })).await.unwrap(),
        json!({})
    );
    let group = rpc(port, "browser.status", json!({ "group_id": "default" })).await.unwrap()["groups"][0].clone();
    assert_eq!(
        (group["controller"].clone(), group["agent_state"].clone(), group["note"].clone(), group["lease"].clone()),
        (json!("user"), json!("stuck"), json!("a captcha"), Value::Null)
    );
    // A lease that ended has no way to the browser any more.
    assert_eq!(reqwest::get(format!("{cdp_url}/json/version")).await.unwrap().status(), reqwest::StatusCode::GONE);

    // The user's side: taken over, the agent gets no lease; handed back, it does.
    let lease = rpc(port, "browser.acquire", json!({ "session_id": "s" })).await.unwrap();
    let taken = rpc(port, "browser.take_over", json!({ "group_id": "default" })).await.unwrap();
    assert_eq!((taken["user_hold"].clone(), taken["controller"].clone(), taken["lease"].clone()), (json!(true), json!("user"), Value::Null));
    let refused = rpc_wire(port, "browser.acquire", json!({ "session_id": "s" })).await.unwrap_err();
    assert_eq!((refused["code"].clone(), refused["kind"].clone()), (json!("user_in_control"), json!("other")), "{refused}");
    let lost = rpc_wire(port, "browser.renew", json!({ "lease_id": lease["lease_id"] })).await.unwrap_err();
    assert_eq!(lost["code"], "lease_lost");
    let back = rpc(port, "browser.hand_back", json!({ "group_id": "default" })).await.unwrap();
    assert_eq!(back["user_hold"], false);
    rpc(port, "browser.acquire", json!({ "session_id": "s" })).await.unwrap();
    // The agent asks for the user.
    let asked = rpc(port, "browser.request_handover", json!({ "session_id": "s", "reason": "password", "note": "the login" })).await.unwrap();
    assert_eq!(asked, json!({ "group_id": "default" }));
    let group = rpc(port, "browser.status", json!({ "group_id": "default" })).await.unwrap()["groups"][0].clone();
    assert_eq!((group["agent_state"].clone(), group["user_hold"].clone()), (json!("waiting_takeover"), json!(true)));
    assert_eq!(group["handover"]["reason"], "password");
    let to_agent = rpc(port, "browser.hand_to_agent", json!({ "group_id": "default" })).await.unwrap();
    assert_eq!((to_agent["user_hold"].clone(), to_agent["handover"].clone()), (json!(false), Value::Null));

    // Errors name what is wrong.
    for (method, params, code) in [
        ("browser.acquire", json!({}), "invalid_argument"),
        ("browser.release", json!({ "lease_id": "l", "state": "happy" }), "invalid_argument"),
        ("browser.request_handover", json!({ "session_id": "s", "reason": "bored" }), "invalid_argument"),
        ("browser.take_over", json!({ "group_id": "_job" }), "no_such_group"),
        ("browser.bind_default", json!({}), "invalid_argument"),
        ("browser.bind_default", json!({ "session_id": "default" }), "invalid_argument"),
        ("browser.renew", json!({ "lease_id": "lease-0" }), "lease_lost"),
        ("browser.upload", json!({ "request_id": "r", "zone_path": "/x" }), "no_such_request"),
        ("browser.upload", json!({ "zone_path": "/x" }), "invalid_argument"),
        ("browser.download_export", json!({ "download_id": "d" }), "no_such_download"),
        ("browser.reflow_state", json!({ "url": "about:blank" }), "invalid_argument"),
        // The reflow library is a bucket of the owner's Zone: this Loader runs outside one.
        ("browser.reflow_state", json!({ "url": "https://example.com" }), "other"),
        ("browser.nope", json!({}), "invalid_argument"),
    ] {
        let error = rpc_wire(port, method, params).await.unwrap_err();
        assert_eq!(error["code"], code, "{method}: {error}");
        assert!(!error["message"].as_str().unwrap().is_empty());
    }

    // The screen and the upload are entries of their own (`the_browsers_screen_and_uploads_are_served_over_http`).
    for (request, status) in [("GET /kapi/opendan/browser/screen", 400), ("PUT /api/opendan/browser/upload/r1?name=a.txt", 404)] {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("{request} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut response)).await.unwrap().unwrap();
        assert!(response.starts_with(&format!("HTTP/1.1 {status}")), "{request}: {response}");
    }

    // Clearing the logins stops the browser; the Loader's end stops it too.
    rpc(port, "browser.clear_profile", json!({})).await.unwrap();
    assert_eq!(rpc(port, "browser.status", json!({})).await.unwrap()["running"], false);
    rpc(port, "browser.ensure", json!({})).await.unwrap();
    let pid = fake.pid().unwrap();
    loader.shutdown().await;
    until("the browser ended with the Loader", || !std::path::Path::new(&format!("/proc/{pid}/cmdline")).exists()).await;
}

/// One HTTP request to the service: `(status, body)`.
#[cfg(unix)]
async fn http_request(port: u16, head: &str, body: &[u8]) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(format!("{head}\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut response)).await.unwrap().unwrap();
    let status = response.split(' ').nth(1).and_then(|code| code.parse().ok()).unwrap_or_else(|| panic!("{response}"));
    let body = response.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or_default();
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[cfg(unix)]
type ScreenSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next message of `kind` on a screen connection (frames are acknowledged), `None` once it is closed.
#[cfg(unix)]
async fn screen_message(ws: &mut ScreenSocket, kind: &str) -> Option<Value> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), ws.next()).await.unwrap_or_else(|_| panic!("no `{kind}` arrived"));
        let Some(Ok(Message::Text(text))) = message else {
            if matches!(message, Some(Ok(Message::Binary(_)))) {
                continue;
            }
            return None;
        };
        let message: Value = serde_json::from_str(text.as_str()).unwrap();
        if message["type"] == kind {
            return Some(message);
        }
        if message["type"] == "frame_meta" {
            ws.send(Message::text(json!({ "type": "ack", "seq": message["seq"] }).to_string())).await.unwrap();
        }
    }
}

/// Connect to the screen below `path` and say `hello`.
#[cfg(unix)]
async fn screen_hello(port: u16, path: &str, token: Option<&str>) -> ScreenSocket {
    use futures::SinkExt;
    let (mut ws, response) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}{path}/browser/screen")).await.unwrap();
    assert_eq!(response.status(), 101);
    let hello = json!({ "type": "hello", "token": token, "group_id": "default", "viewport": { "width": 412, "height": 915, "mobile": true } });
    ws.send(tokio_tungstenite::tungstenite::Message::text(hello.to_string())).await.unwrap();
    ws
}

/// Browser TODO SC1 / SC6 through the Loader: `GET …/browser/screen` below
/// every mount becomes a WebSocket whose `hello` is admitted like a kRPC
/// call of the owner's, and `PUT …/browser/upload/<request>` gives a file
/// of the App's device to the page that asked for one.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browsers_screen_and_uploads_are_served_over_http() {
    let w = World::new();
    let fake = fake_browser_of(&w).await;
    let port = free_port();
    let loader = Loader::start(w.env(Llm::new(|_, _| "unused".into()), port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    let agent_path = format!("/kapi/{}", loader.agent.agent_id());

    // Outside a Zone every caller is the owner: the hello is answered with the group's state.
    for path in [WEB_API_PATH, SERVICE_PATH, agent_path.as_str()] {
        let mut ws = screen_hello(port, path, None).await;
        let ready = screen_message(&mut ws, "ready").await.unwrap();
        assert_eq!(ready["state"]["group"]["group_id"], "default", "{path}");
        // A phone's viewport: the browser is not made a phone.
        assert_eq!((ready["state"]["running"].clone(), ready["state"]["viewport"]["mobile"].clone()), (json!(true), json!(false)));
        assert_eq!(loader.browser.viewers(), 1);
        drop(ws);
        until("the viewer is gone", || loader.browser.viewers() == 0).await;
    }
    assert!(fake.pid().is_some(), "the browser was started for the screen");
    // Not a WebSocket request: refused, nothing is upgraded.
    let (status, error) = http_request(port, &format!("GET {SERVICE_PATH}/browser/screen HTTP/1.1"), b"").await;
    assert_eq!((status, error["code"].as_str()), (400, Some("invalid_argument")));

    // A page asks for a file: the screen is told, the App uploads it, the page's input gets it.
    let tab = loader.browser.open_tab("default", Some("https://a.example/")).await.unwrap();
    let (_, session) = loader.browser.tab_session(&tab.tab_id).await.unwrap();
    let mut ws = screen_hello(port, SERVICE_PATH, None).await;
    screen_message(&mut ws, "ready").await.unwrap();
    fake.tell(&tab.tab_id, "Page.fileChooserOpened", json!({ "frameId": tab.tab_id, "mode": "selectMultiple", "backendNodeId": 9 }));
    let request = screen_message(&mut ws, "request").await.unwrap();
    assert_eq!((request["kind"].clone(), request["multiple"].clone(), request["accept"].clone()), (json!("file_chooser"), json!(true), json!([".png", "image/*"])));
    let request_id = request["request_id"].as_str().unwrap();
    let put = |query: &str, length: usize| format!("PUT {agent_path}/browser/upload/{request_id}?{query} HTTP/1.1\r\nContent-Length: {length}");
    assert_eq!(http_request(port, &put("name=first%20file.txt", 5), b"first").await, (200, json!({})));
    assert!(fake.received("DOM.setFileInputFiles").is_empty(), "the files are given together");
    // What cannot be taken says why.
    let (status, error) = http_request(port, &put("done=1", 1), b"x").await;
    assert_eq!((status, error["code"].as_str()), (400, Some("invalid_argument")), "a file has a name");
    let (status, error) = http_request(port, &put("name=huge.bin&done=1", 200 * 1024 * 1024), b"").await;
    assert_eq!((status, error["code"].as_str()), (413, Some("upload_too_large")));
    let (status, error) = http_request(port, &format!("PUT {SERVICE_PATH}/browser/upload/r-none?name=a.txt HTTP/1.1\r\nContent-Length: 1"), b"x").await;
    assert_eq!((status, error["code"].as_str()), (404, Some("no_such_request")));
    assert_eq!(http_request(port, &put("name=%E7%85%A7%E7%89%87.png&done=1", 6), b"second").await, (200, json!({})));
    let set = fake.received("DOM.setFileInputFiles");
    assert_eq!((set.len(), set[0].0.as_deref(), set[0].1["backendNodeId"].clone()), (1, Some(session.as_str()), json!(9)));
    let files: Vec<PathBuf> = set[0].1["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect();
    assert_eq!(files.iter().map(|f| f.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), vec!["first file.txt", "照片.png"]);
    assert_eq!((std::fs::read(&files[0]).unwrap(), std::fs::read(&files[1]).unwrap()), (b"first".to_vec(), b"second".to_vec()));
    assert!(files[0].starts_with(w.root.join(".cache/browser/uploads")));
    assert_eq!(screen_message(&mut ws, "request_closed").await.unwrap()["request_id"], request_id);
    // The request is over: nothing more is taken for it.
    assert_eq!(http_request(port, &put("name=late.txt&done=1", 1), b"x").await.0, 404);
    // Over the connection the user operates the page.
    futures::SinkExt::send(&mut ws, tokio_tungstenite::tungstenite::Message::text(json!({ "type": "input", "kind": "text", "text": "typed" }).to_string())).await.unwrap();
    until("the input reached the tab", || fake.received("Input.insertText").iter().any(|(s, p)| s.as_deref() == Some(session.as_str()) && p["text"] == "typed")).await;
    drop(ws);
    loader.shutdown().await;

    // Inside a Zone the hello and the upload carry the owner's session token, checked like a kRPC call's.
    let w = World::new();
    let fake = fake_browser_of(&w).await;
    let port = free_port();
    let mut env = w.env(Llm::new(|_, _| "unused".into()), port).await;
    env.access = Access::Zone { owner: "bob".into(), app_id: "jarvis.alice".into(), trust_loopback: false, zone_host: Some("test.buckyos.io".into()) };
    let loader = Loader::start(env).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    for token in [None, Some("not-a-session-token")] {
        let mut ws = screen_hello(port, SERVICE_PATH, token).await;
        let error = screen_message(&mut ws, "error").await.unwrap();
        assert_eq!(error["code"], "unauthorized", "{token:?}: {error}");
        assert!(!error["message"].as_str().unwrap().is_empty());
        assert!(screen_message(&mut ws, "ready").await.is_none(), "the connection is closed");
    }
    let (status, error) = http_request(port, &format!("PUT {SERVICE_PATH}/browser/upload/r1?name=a.txt HTTP/1.1\r\nContent-Length: 1"), b"x").await;
    assert_eq!((status, error["code"].as_str()), (401, Some("unauthorized")));
    let (status, error) =
        http_request(port, &format!("PUT {SERVICE_PATH}/browser/upload/r1?name=a.txt HTTP/1.1\r\nAuthorization: Bearer not-a-session-token\r\nContent-Length: 1"), b"x").await;
    assert_eq!((status, error["code"].as_str()), (403, Some("unauthorized")));
    assert!(fake.pid().is_none() && loader.browser.viewers() == 0, "nobody was admitted");
    loader.shutdown().await;
}

/// An agent without a browser says why on every entry: `browser.status`,
/// the error of a kRPC call, the screen's handshake and the upload carry
/// `unavailable_code` beside `browser_unavailable`.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_without_a_browser_says_why_on_every_entry() {
    let w = World::new();
    std::fs::write(w.root.join("agent.toml"), format!("{AGENT_TOML}enabled = false\n")).unwrap();
    let fake = fake_browser_of(&w).await;
    let port = free_port();
    let loader = Loader::start(w.env(Llm::new(|_, _| "unused".into()), port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;

    let status = rpc(port, "browser.status", json!({})).await.unwrap();
    assert_eq!((status["enabled"].clone(), status["available"].clone(), status["unavailable_code"].clone()), (json!(false), json!(true), json!("disabled")));
    for (method, params) in [
        ("browser.ensure", json!({})),
        ("browser.acquire", json!({ "session_id": "s" })),
        ("browser.take_over", json!({ "group_id": "default" })),
        ("browser.page_hint", json!({})),
    ] {
        let error = rpc_wire(port, method, params).await.unwrap_err();
        assert_eq!(
            (error["kind"].as_str(), error["code"].as_str(), error["unavailable_code"].as_str()),
            (Some("other"), Some("browser_unavailable"), Some("disabled")),
            "{method}: {error}"
        );
        assert_eq!(error["message"], status["unavailable"], "{method}");
    }
    let mut ws = screen_hello(port, SERVICE_PATH, None).await;
    let error = screen_message(&mut ws, "error").await.unwrap();
    assert_eq!((error["code"].as_str(), error["unavailable_code"].as_str()), (Some("browser_unavailable"), Some("disabled")), "{error}");
    assert!(screen_message(&mut ws, "ready").await.is_none(), "the connection is closed");
    let (code, error) = http_request(port, &format!("PUT {SERVICE_PATH}/browser/upload/r1?name=a.txt HTTP/1.1\r\nContent-Length: 1"), b"x").await;
    assert_eq!((code, error["code"].as_str(), error["unavailable_code"].as_str()), (503, Some("browser_unavailable"), Some("disabled")), "{error}");
    assert!(fake.pid().is_none(), "nothing was started");
    loader.shutdown().await;
}

/// Browser TODO BR7 / HO4, D35, D36: every session works in the agent's one
/// group. The browser conversation is the one `browser.bind_default` names:
/// what the user does to the browser is told to its UI session as a
/// `_browser` observation — it says nothing by itself, only the latest one is
/// kept, and the owner's next message sees it — and its questions are what
/// the browser shows the agent waiting for. Its UI sessions have the
/// browser's manual, no other conversation's has; a UI session it had before
/// it was bound is archived, and its next message opens one with the manual.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn taking_the_browser_over_is_told_to_the_browser_conversation() {
    use libopendan::notify::NotifyHost;
    let mut w = World::new();
    std::fs::create_dir_all(w.root.join("cmd_skills/browser")).unwrap();
    std::fs::write(w.root.join("cmd_skills/browser/SKILL.md"), "---\ndescription: the browser\nscope: ui, work\n---\nThe browser manual.").unwrap();
    let notify = Arc::new(libopendan::notify::LocalNotify::enable(&w.root, AGENT).unwrap());
    w.notify = Some(notify.clone());
    let fake = fake_browser_of(&w).await;
    let port = free_port();
    let llm = Llm::new(|req, n| {
        let all: String = req.messages.iter().map(|m| m.text_content()).collect();
        let manual = all.contains("The browser manual.");
        match n {
            0 | 1 => {
                assert!(!manual, "no conversation has the browser's manual before it is the browser conversation");
                "hello".to_string()
            }
            2 => {
                assert!(manual, "the browser conversation's new session has the manual");
                "ready".to_string()
            }
            3 => {
                assert!(all.contains("[browser] The user handed the browser back."), "{all}");
                assert!(all.contains("\"Title of https://shop.example/cart\" (https://shop.example/cart)"), "{all}");
                assert!(!all.contains("The user took over the browser"), "only the latest record is kept: {all}");
                "reading the page again".to_string()
            }
            4 => {
                assert!(all.contains("[browser] The user asks you to go on from the current page"), "{all}");
                "on it".to_string()
            }
            5 => {
                assert!(manual, "{all}");
                "here too".to_string()
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let loader = Loader::start(w.env(llm.clone(), port).await).await.unwrap();
    until("service answers", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()).await;
    let route = |session: &str| MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session.to_string())).unwrap().to_string();
    let open_ui = |session: &str| {
        let entries = futures_block(loader.agent.sessions().query(&RegistryQuery { kind: Some(SessionKind::Ui), ..Default::default() })).unwrap();
        entries.into_iter().find(|e| e.route_key.as_deref() == Some(route(session).as_str()) && e.status.run_state != RunState::Finished)
    };
    let manuals = |entry: &RegistryEntry| {
        let cfg = SessionDir::open(&entry.location).unwrap().config().unwrap();
        cfg.prompt.frozen.unwrap().cmd_skills.into_iter().map(|c| c.name).collect::<Vec<_>>()
    };
    let pending = |entry: &RegistryEntry| SessionDir::open(&entry.location).and_then(|sd| sd.state()).map(|st| st.pending_events).unwrap();
    let say = |session: &str, text: &str| {
        let mut msg = chat(BOB, text);
        msg.to_session = Some(session.to_string());
        w.mail.deliver(session, msg);
    };

    // Two conversations of the owner, neither of them the browser conversation yet.
    let direct = format!("dm:{BOB}");
    w.mail.deliver(&direct, chat(BOB, "hi"));
    until("first reply", || w.mail.sent().len() == 1).await;
    say("thing-1", "book the flight");
    until("second reply", || w.mail.sent().len() == 2).await;
    let ui_direct = open_ui(&direct).unwrap();
    let before = open_ui("thing-1").unwrap();
    assert!(manuals(&ui_direct).is_empty() && manuals(&before).is_empty());

    // Every session works in the one group, also one the registry does not know.
    for session in [ui_direct.session_id.as_str(), before.session_id.as_str(), "no-such-session"] {
        let lease = rpc(port, "browser.acquire", json!({ "session_id": session })).await.unwrap();
        assert_eq!(lease["group_id"], "default", "session {session}");
        rpc(port, "browser.release", json!({ "lease_id": lease["lease_id"] })).await.unwrap();
        let status = rpc(port, "browser.status", json!({ "session_id": session })).await.unwrap();
        assert_eq!(status["groups"].as_array().unwrap().iter().map(|g| g["group_id"].clone()).collect::<Vec<_>>(), vec![json!("default")]);
    }
    for (method, params) in [("browser.status", json!({ "group_id": "thing-1" })), ("browser.take_over", json!({ "group_id": "thing-1" }))] {
        assert_eq!(rpc_wire(port, method, params).await.unwrap_err()["code"], "no_such_group", "{method}");
    }
    // Without a browser conversation the browser changes hands without a record.
    let silent = rpc(port, "browser.take_over", json!({ "group_id": "default" })).await.unwrap();
    assert_eq!(silent["user_hold"], true);
    rpc(port, "browser.hand_back", json!({ "group_id": "default" })).await.unwrap();
    assert!(pending(&ui_direct).is_empty() && pending(&before).is_empty());

    // The named conversation becomes the browser conversation: its session without the manual is archived.
    assert_eq!(rpc(port, "browser.bind_default", json!({ "session_id": "thing-1" })).await.unwrap(), json!({ "default_session_id": "thing-1" }));
    assert_eq!(rpc(port, "browser.status", json!({})).await.unwrap()["default_session_id"], "thing-1");
    until("the session without the manual is archived", || open_ui("thing-1").is_none()).await;
    say("thing-1", "the browser is yours");
    until("third reply", || w.mail.sent().len() == 3).await;
    let ui_browser = open_ui("thing-1").unwrap();
    assert_ne!(ui_browser.session_id, before.session_id);
    assert_eq!(manuals(&ui_browser), ["browser"]);
    assert!(manuals(&open_ui(&direct).unwrap()).is_empty(), "another conversation has no browser manual");
    // Bound again: nothing is archived.
    rpc(port, "browser.bind_default", json!({ "session_id": "thing-1" })).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(open_ui("thing-1").unwrap().session_id, ui_browser.session_id);

    // The user opens a page, takes over, hands back: the browser conversation is told, no other.
    let tab = loader.browser.open_tab("default", Some("https://shop.example/")).await.unwrap();
    loader.browser.tab_send(&tab.tab_id, "Page.navigate", json!({ "url": "https://shop.example/cart" })).await.unwrap();
    until("the page's title arrives", || loader.browser.group("default").tabs[0].title.contains("cart")).await;
    rpc(port, "browser.take_over", json!({ "group_id": "default" })).await.unwrap();
    until("the take-over is observed", || pending(&ui_browser).len() == 1).await;
    let seen = pending(&ui_browser);
    assert_eq!(seen[0].subscription_id.as_deref(), Some("_browser"));
    assert_eq!((seen[0].source.kind.as_str(), seen[0].source.id.as_str()), ("system", "browser:default"));
    assert_eq!(seen[0].latest.as_ref().unwrap().event, "take_over");
    rpc(port, "browser.hand_back", json!({ "group_id": "default" })).await.unwrap();
    until("the hand-back replaces it", || pending(&ui_browser)[0].latest.as_ref().is_some_and(|l| l.event == "hand_back")).await;
    assert_eq!(pending(&ui_browser).len(), 1);
    assert!(pending(&ui_direct).is_empty(), "another conversation is not told");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 3, "an observation says nothing");
    // The App wakes the agent with an ordinary message.
    say("thing-1", "go on");
    until("fourth reply", || w.mail.sent().len() == 4).await;
    assert_eq!(w.mail.sent()[3].1.content.content, "reading the page again");

    // A question the agent asked in the browser conversation (here: a Work of it) shows as its state; one asked elsewhere does not.
    let agent_state = || async move { rpc(port, "browser.status", json!({})).await.unwrap()["groups"][0]["agent_state"].clone() };
    let work_of = |parent: &RegistryEntry, objective: &str| {
        let mut spec = work_spec(objective, "go");
        spec.origin = Some(Origin { parent_session: Some(parent.session_id.clone()), ..Default::default() });
        spec.driver = Some("app:other@alice".into());
        spec
    };
    let elsewhere = create_session(&w.root.join("sessions"), work_of(&ui_direct, "a work of the direct conversation"), loader.agent.as_ref(), WHO, w.channels().as_ref()).await.unwrap();
    let work = create_session(&w.root.join("sessions"), work_of(&ui_browser, "a work of the browser conversation"), loader.agent.as_ref(), WHO, w.channels().as_ref()).await.unwrap();
    assert_eq!(agent_state().await, "idle", "a conversation waiting for its next message is not a question");
    let ask = |session: &str, key: &str| {
        let mut question = buckyos_api::NotifySendReq::input("", "Window or aisle?", "", Vec::new(), true, key);
        question.source = Some(buckyos_api::NotifySource { session_id: Some(session.to_string()), ..Default::default() });
        question
    };
    let other = notify.send(ask(elsewhere.sid(), "ask-elsewhere")).await.unwrap();
    assert_eq!(agent_state().await, "idle", "asked in another conversation");
    let question = notify.send(ask(work.sid(), "ask-seat")).await.unwrap();
    assert_eq!(agent_state().await, "waiting_answer");
    let lease = rpc(port, "browser.acquire", json!({ "session_id": work.sid() })).await.unwrap();
    assert_eq!(agent_state().await, "working", "an agent at work is shown working");
    rpc(port, "browser.release", json!({ "lease_id": lease["lease_id"], "state": "done" })).await.unwrap();
    assert_eq!(agent_state().await, "waiting_answer", "the open question comes before how the last lease ended");
    notify.cancel(&question.notify_id, "answered in the test").await.unwrap();
    notify.cancel(&other.notify_id, "answered in the test").await.unwrap();
    assert_eq!(agent_state().await, "done");

    // Handed to the agent: the browser conversation is told, and the App's message wakes it.
    let handed = rpc(port, "browser.hand_to_agent", json!({ "group_id": "default" })).await.unwrap();
    assert_eq!(handed["group_id"], "default");
    until("the hand-over is observed", || pending(&ui_browser).len() == 1).await;
    say("thing-1", "from here");
    until("fifth reply", || w.mail.sent().len() == 5).await;

    // Another conversation becomes the browser conversation: its first session has the manual; the tabs stay.
    rpc(port, "browser.bind_default", json!({ "session_id": "thing-3" })).await.unwrap();
    say("thing-3", "hello browser");
    until("sixth reply", || w.mail.sent().len() == 6).await;
    assert_eq!(manuals(&open_ui("thing-3").unwrap()), ["browser"]);
    assert_eq!(open_ui("thing-1").unwrap().session_id, ui_browser.session_id, "the conversation bound before keeps its session");
    let status = rpc(port, "browser.status", json!({})).await.unwrap();
    assert_eq!((status["default_session_id"].clone(), status["groups"][0]["tabs"].as_array().unwrap().len()), (json!("thing-3"), 1));
    let file: Value = serde_json::from_slice(&std::fs::read(w.root.join("state/browser/groups.json")).unwrap()).unwrap();
    assert_eq!(file["default_session_id"], "thing-3");
    drop(fake);
    loader.shutdown().await;
}
