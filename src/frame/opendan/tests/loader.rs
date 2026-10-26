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
    TaskWaitReason,
};
use libopendan::api::create_session;
use libopendan::channel::{KmsgChannels, PollWaker};
use libopendan::protocol::*;
use libopendan::runner::RunnerOptions;
use libopendan::state::krpc::{KrpcTransport, StateTransport};
use libopendan::state::{connect, AgentStateClient, ConnectOptions, StateLocator};
use libopendan::{OpenDanError, SessionDir, SessionTemplate};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::{LLMComputeError, ProviderFailure, RefusalKind};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, ObjId};
use opendan::loader::{Loader, LoaderEnv, SpecRef};
use opendan::records::{self, AgentRecords, LocalRecords};
use opendan::service::{Access, SERVICE_PATH, WEB_API_PATH};
use opendan::tasks::{NewTask, TaskService, TaskView};
use opendan::ui::MailService;
use serde_json::{json, Value};

const AGENT: &str = "did:bns:jarvis.alice";
const WHO: &str = "app:jarvis@alice";
const BOB: &str = "did:bns:bob";

struct Llm {
    calls: AtomicUsize,
    /// Calls that fail before the script answers.
    failures: AtomicUsize,
    /// What the failing calls are: a provider error, or this policy refusal.
    refusal: std::sync::Mutex<Option<RefusalKind>>,
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
                failure: refusal.map(ProviderFailure::Refused).unwrap_or(ProviderFailure::Unknown),
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

/// M1: sessions registered for this driver are hosted without anything
/// touching them — those left over from before the start and those created
/// while the Loader runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registered_sessions_are_hosted_and_recovered() {
    let w = World::new();
    std::fs::create_dir_all(w.root.join("sessions")).unwrap();
    let before = new_work(fs_agent(&w).as_ref(), &w, "left from an earlier process").await;
    assert_eq!(before.state().unwrap().run_state, RunState::Created);

    let loader = w.start(Llm::new(|_, _| "done".into())).await;
    until("recovered session finishes", || finished(&before)).await;
    let later = new_work(loader.agent.as_ref(), &w, "created while hosting").await;
    until("new session finishes", || finished(&later)).await;
    assert_eq!(later.state().unwrap().outcome, Some(Outcome::Succeeded));
    let hosted = loader.supervisor.status();
    for sd in [&before, &later] {
        let h = hosted.iter().find(|h| h.session_id == sd.sid()).unwrap();
        assert!(h.drives >= 1, "{h:?}");
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
    assert_eq!(
        remote.sessions().lookup(done.sid()).await.unwrap(),
        local.sessions().lookup(done.sid()).await.unwrap()
    );
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
    assert_eq!(modules["self_check"], false);
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
    assert_eq!(known[0]["location"]["directory"], json!(directory));
    assert_eq!(known[0]["lifecycle"], "archived");
    assert_eq!(known[0]["availability"], "missing");
    assert!(known[0]["last_error"].is_string());
    assert!(known[0].get("private_notes").is_none());
    assert!(external.path().join("moved-project").exists());

    // The three operations are records on the session's bus.
    let posted = rpc(port, "session.post", json!({ "sid": ui.sid(), "text": "second" })).await.unwrap();
    assert!(posted["key"].as_str().unwrap().starts_with("cymsg:"));
    until("second turn", || ui.state().map(|s| s.turns_completed == 2).unwrap_or(false)).await;
    rpc(port, "session.stop", json!({ "sid": ui.sid(), "reason": "enough" })).await.unwrap();
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

fn futures_block<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

/// M3: a message in an inbox creates the UI session bound to that inbox;
/// the reply goes back to the speaker, in the same conversation, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbox_messages_are_answered_along_the_same_conversation() {
    let w = World::new();
    let llm = Llm::new(|req, n| format!("answer {n} to: {}", last_user(req).contains("hello")));
    let loader = w.start(llm.clone()).await;
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

    // `/stop` from the owner ends the session; the next message of that
    // inbox opens the next generation.
    w.mail.deliver(&session, chat(BOB, "/stop"));
    until("stopped", || sd.state().map(|s| s.is_finished()).unwrap_or(false)).await;
    w.mail.deliver(&session, chat(BOB, "hello once more"));
    until("reply of the next generation", || w.mail.sent().len() == 3).await;
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
    *llm.refusal.lock().unwrap() = Some(RefusalKind::QuotaExhausted);
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
            "there are 3 files".to_string()
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
    let mut env = w.env(llm, free_port()).await;
    env.package_root = Some(package);
    let loader = Loader::start(env).await.unwrap();
    assert_eq!(loader.config.language, "zh");
    assert_eq!(loader.config.llm_context["provider"]["type"], "buckyos");
    for name in ["chat_route", "plan", "do", "work_simple", "goal", "plan_simple"] {
        let b = loader.agent.behaviors().get(name).await.unwrap().unwrap();
        assert!(b.prompt.system.is_some(), "{name}");
    }
    // plan_simple: native function calls with an intent on every call and
    // the background after tool results; its session class ends with an
    // explicit report.
    let plan_simple = loader.agent.behaviors().get("plan_simple").await.unwrap().unwrap();
    let entry = plan_simple.to_entry("plan_simple", &loader.config.llm_context, true).unwrap();
    assert!(entry.call_intent);
    assert_eq!(entry.input.unwrap().background, libopendan::protocol::InputBackground::AfterTools);
    assert!(loader.agent.behaviors().get("task_route").await.unwrap().is_none());
    // Intent analysis: a fork of the UI context offered as a named function.
    let analyze = loader.agent.behaviors().get("analyze_intent").await.unwrap().unwrap();
    assert!(analyze.prompt.system.is_none() && analyze.model.preferred.is_none());
    assert!(analyze.prompt.on_context_switch.as_deref().unwrap().contains("agent-session ask-user"));
    let entry = analyze.to_entry("analyze_intent", &loader.config.llm_context, false).unwrap();
    assert_eq!(entry.mode, libopendan::protocol::ContextMode::Fork);
    assert_eq!(entry.tool.as_ref().unwrap().function_name("analyze_intent"), "try_analyze_user_intent");
    assert_eq!(
        entry.tool.unwrap().render_task(&serde_json::json!({ "request": "tidy the docs" })),
        "tidy the docs"
    );
    let simple = loader.agent.behaviors().get("work_simple").await.unwrap().unwrap();
    assert_eq!(simple.capabilities.tool_whitelist.as_deref(), Some(&["group:bash".to_string(), "report".to_string()][..]));
    // The built-in runtimes are registered before anything runs.
    let runtimes = loader.agent.runtimes().list().await.unwrap();
    let ids: Vec<_> = runtimes.iter().map(|r| (r.runtime_id.as_str(), r.availability)).collect();
    assert_eq!(
        ids,
        [
            ("container", libopendan::protocol::RuntimeAvailability::Available),
            ("host", libopendan::protocol::RuntimeAvailability::RuntimeUnavailable)
        ]
    );
    // Both conversations reply under a contract; the analysis returns its
    // result under its own, listing the questions it asked.
    for name in ["chat_route", "groupchat_route"] {
        let b = loader.agent.behaviors().get(name).await.unwrap().unwrap();
        let reply = b.reply.as_ref().unwrap_or_else(|| panic!("{name} has no [reply]"));
        assert_eq!(reply.schema["required"], serde_json::json!(["reply", "topic", "artifacts"]), "{name}");
        assert_eq!(
            b.capabilities.tool_whitelist.as_deref(),
            Some(&["group:bash_core".to_string(), "behavior:analyze_intent".to_string()][..]),
            "{name}"
        );
    }
    assert_eq!(analyze.reply.as_ref().unwrap().asks_field.as_deref(), Some("clarifications"));
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
    assert!(system.contains("## Commands\nRun these through `shell`"), "the cmd_skill is frozen: {system}");
    assert!(!system.contains("requires: an OpenDAN UI session"), "front matter is not part of the manual: {system}");
    assert!(!system.contains("<<"), "{system}");
    let mut tools = tools.lock().unwrap().clone();
    tools.sort();
    assert_eq!(tools, ["edit_file", "read", "shell", "try_analyze_user_intent", "write_file"]);
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
    // A work session of the package starts in plan and can hand over to do.
    let template = SessionTemplate::load("work", loader.agent.agent_root()).unwrap();
    assert_eq!(template.default_behavior.as_deref(), Some("plan"));
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
            return r#"REPORT:{"report":"已完成 PR #667 的审查，发现 1 项 P2（中等）问题：创建响应丢失时清理逻辑会遗留临时账户。","is_end":true}"#
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
    assert_eq!(tools, ["edit_file", "read", "shell", "try_analyze_user_intent", "write_file"]);
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
    std::fs::create_dir_all(package.join("cmd_skills")).unwrap();
    std::fs::write(package.join("role.md"), "role v1").unwrap();
    std::fs::write(package.join("cmd_skills/tip.md"), "tip v1").unwrap();
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
    std::fs::write(package.join("cmd_skills/tip.md"), "tip v2").unwrap();
    let mut stale = info.clone();
    stale.loaded_at = 0;
    w.records
        .put(records::INFO, serde_json::to_value(&stale).unwrap())
        .await
        .unwrap();
    let loader = start(&w, &package, "2.0.0").await;
    assert_eq!(std::fs::read_to_string(w.root.join("role.md")).unwrap(), "role v1");
    assert_eq!(std::fs::read_to_string(w.root.join("cmd_skills/tip.md")).unwrap(), "tip v1");
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
    assert_eq!(task.new.idempotency_key, format!("agent_turn:{}:1", entry.session_id));
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
            "part done".to_string()
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
async fn a_canceled_task_stops_its_session() {
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
    until("the session heard the stop", || {
        sd.state().map(|s| s.stop_requested || s.is_finished()).unwrap_or(false)
    })
    .await;
    release.store(true, Ordering::SeqCst);
    until("stopped", || finished(&sd)).await;
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
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

// Intent analysis: asking the user from a fork (TODO V16) ------------------

const ANALYZE_TOML: &str = r#"
[meta]
name = "analyze"
[entry]
mode = "fork"
[tool]
name = "try_analyze"
description = "Analyze the request."
task = "{{args.request}}"
[tool.parameters]
type = "object"
required = ["request"]
[tool.parameters.properties.request]
type = "string"
[prompt]
on_context_switch = "ANALYZE: {{handover.sub_task.task}}"
"#;

const CHAT_TOML: &str = r#"
[meta]
name = "chat"
next = ["analyze"]
[prompt]
mode = "function_call"
system = "chat with the owner"
[capabilities]
tool_whitelist = ["group:bash", "behavior:analyze", "report"]
"#;

/// The answer `ask-user` would print (the command itself is tested with
/// libopendan; this host has no `agent-session`).
fn ask_command(question: &str, input: Value) -> String {
    let out = json!({
        "agent_tool_protocol": "1", "tool": "ask-user", "status": "pending",
        "task_id": "userinput:%s", "summary": "waiting for the answer",
        "detail": { "question": question, "input": input },
    });
    format!("TOOL:printf '{}' \"$XLLM_CALL_ID\"", out)
}

fn qa_world() -> World {
    let w = World::with_tasks();
    std::fs::create_dir_all(w.root.join("behaviors")).unwrap();
    std::fs::write(w.root.join("behaviors/analyze.toml"), ANALYZE_TOML).unwrap();
    std::fs::write(w.root.join("behaviors/chat.toml"), CHAT_TOML).unwrap();
    std::fs::write(
        w.root.join("agent.toml"),
        format!("[loader]\nplaceholder_delay_ms = 60000\n{AGENT_TOML}\n[session.ui]\ndefault_behavior = \"chat\"\n"),
    )
    .unwrap();
    w
}

fn thread_reply(text: &str, target: &MsgObject, machine: Option<Value>) -> MsgObject {
    let mut m = chat(BOB, text);
    m.relates_to = Some(ndn_lib::MsgRelation::new(
        ndn_lib::MsgRelType::Thread,
        ObjId::new(&msg_key(target)).unwrap(),
    ));
    m.content.machine = machine.map(|v| serde_json::from_value(v).unwrap());
    m
}

/// V16: the user gets the placeholder P at once, the question Q as a thread
/// message on P carrying `agent_ask`, answers on P, and the Turn's result R
/// as the edit of P — one Turn, one task, which waits for the answer in
/// between.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_and_its_answer_stay_in_the_turn() {
    let w = qa_world();
    let tasks = w.tasks.clone().unwrap();
    let llm = Llm::new(|req, n| match n {
        0 => r#"CALL:try_analyze {"request":"tidy up the docs"}"#.to_string(),
        1 => {
            assert!(last_user(req).contains("ANALYZE: tidy up the docs"), "{}", last_user(req));
            ask_command(
                "docs/ or everything?",
                json!({ "kind": "choice", "options": [ { "id": "a", "label": "docs/" }, { "id": "b", "label": "everything" } ] }),
            )
        }
        2 => {
            let all: String = req.messages.iter().map(|m| serde_json::to_string(m).unwrap()).collect();
            assert!(all.contains(r#"\"option\":\"a\""#) && all.contains(r#"\"in_reply_to_question\":true"#), "{all}");
            r#"REPORT:{"report":"created work for docs/","result":{"outcome":"work_created"},"is_end":true}"#.to_string()
        }
        3 => {
            let all: String = req.messages.iter().map(|m| serde_json::to_string(m).unwrap()).collect();
            assert!(all.contains("work_created"), "{all}");
            "Started: tidying docs/.".to_string()
        }
        _ => panic!("unexpected call {n}"),
    });
    let loader = w.start(llm.clone()).await;
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "please tidy up the docs"));
    until("placeholder and question", || w.mail.sent().len() == 2).await;
    let sent = w.mail.sent();
    let (p, q) = (&sent[0].1, &sent[1].1);
    assert_eq!(p.content.content, "(working)");
    let task_id = agent_task_of(p).expect("P carries the Turn's task");
    let rel = q.relates_to.as_ref().expect("Q is a thread message on P");
    assert_eq!((rel.rel.clone(), rel.target.to_string()), (ndn_lib::MsgRelType::Thread, msg_key(p)));
    assert_eq!(q.meta["agent_ask"]["anchor"], msg_key(p).as_str());
    assert_eq!(q.meta["agent_ask"]["input"]["kind"], "choice");
    assert!(q.content.content.starts_with("docs/ or everything?\n1. docs/"), "{}", q.content.content);
    assert!(q.mentions.is_none(), "a direct chat mentions nobody");
    until("task waits for the answer", || {
        tasks.all().iter().any(|t| t.id == task_id && t.state == "waiting" && t.message.contains("answer"))
    })
    .await;

    w.mail.deliver(
        &session,
        thread_reply("docs/", p, Some(json!({ "intent": "agent_ask.answer", "data": { "call_id": "c1", "option": "a" } }))),
    );
    until("result", || w.mail.sent().len() == 3).await;
    let r = &w.mail.sent()[2].1;
    assert_eq!(r.content.content, "Started: tidying docs/.");
    let rel = r.relates_to.as_ref().expect("R edits P");
    assert_eq!((rel.rel.clone(), rel.target.to_string()), (ndn_lib::MsgRelType::Edit, msg_key(p)));
    until("task closed", || tasks.all().iter().any(|t| t.id == task_id && t.state == "succeeded")).await;
    assert_eq!(tasks.all().len(), 1, "one Turn, one task");
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session)).unwrap().to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let st = SessionDir::open(&entry.location).unwrap().state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert_eq!(llm.calls.load(Ordering::SeqCst), 4);
    loader.shutdown().await;
}

/// In a group the question is addressed to the owner (MessageHub locks the
/// composer of the members a question mentions).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_in_a_group_mentions_the_owner() {
    let w = qa_world();
    let mut settings = w.settings.clone();
    settings.allow_group = true;
    let w = World { settings, ..w };
    let llm = Llm::new(|_req, n| match n {
        0 => r#"CALL:try_analyze {"request":"plan the trip"}"#.to_string(),
        1 => ask_command("which weekend?", json!({ "kind": "text" })),
        _ => "ok".to_string(),
    });
    let loader = w.start(llm).await;
    let group = "did:bns:hikers";
    w.mail.deliver(group, group_msg(BOB, group, "@jarvis plan the trip", true));
    until("question", || w.mail.sent().iter().any(|(_, m)| m.meta.contains_key("agent_ask"))).await;
    let q = w.mail.sent().into_iter().map(|(_, m)| m).find(|m| m.meta.contains_key("agent_ask")).unwrap();
    assert_eq!(q.kind, MsgObjKind::GroupMsg);
    assert_eq!(q.mentions.as_ref().unwrap().dids, vec![parse_did(BOB).unwrap()]);
    loader.shutdown().await;
}

/// Notify TODO V07 / V08 through the Loader: a Work's question is an
/// `input` notification, the Turn's task waits for the user (HumanInput on
/// the notification's task), the answer continues the Work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_work_question_is_a_notification_and_its_answer_continues_the_work() {
    use libopendan::notify::{LocalNotify, NotifyAnswer, NotifyHost, NotifyListReq};
    let mut w = World::with_tasks();
    let local = Arc::new(LocalNotify::enable(&w.root, AGENT).unwrap());
    w.notify = Some(local.clone());
    let tasks = w.tasks.clone().unwrap();
    let llm = Llm::new(|req, n| match n {
        0 => ask_command(
            "Postgres or SQLite?",
            json!({ "kind": "choice", "options": [ { "id": "pg", "label": "Postgres" }, { "id": "lite", "label": "SQLite" } ] }),
        ),
        1 => {
            let all: String = req.messages.iter().map(|m| serde_json::to_string(m).unwrap()).collect();
            assert!(all.contains(r#"\"option\":\"pg\""#) && all.contains(r#"\"via\":\"notification\""#), "{all}");
            "migrated to Postgres".to_string()
        }
        _ => panic!("unexpected call {n}"),
    });
    let loader = w.start(llm.clone()).await;
    let sd = new_work(loader.agent.as_ref(), &w, "migrate the notes").await;
    let pending = || futures_block(local.list(NotifyListReq::default())).unwrap();
    until("the question", || pending().len() == 1).await;
    let v = pending().remove(0);
    let human = v.task_id.clone().unwrap();
    until("the Turn waits for the user", || {
        tasks.waits.lock().unwrap().iter().any(|(_, r)| {
            r.kind == buckyos_api::TaskWaitReasonKind::HumanInput
                && r.related_task_id.as_deref() == Some(human.as_str())
                && r.code.as_deref() == Some("ask_user")
        })
    })
    .await;
    local.answer(&v.notify_id, NotifyAnswer::option("pg"), None).await.unwrap();
    until("the work finishes", || finished(&sd)).await;
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
    assert!(sd.report().unwrap().contains("migrated to Postgres"));
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
    for name in ["xagent", "agent-session"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_opendan"), bin.join(name)).unwrap();
    }
    let mut env = w.env(Llm::new(|_, _| "done".into()), port).await;
    env.session_cli = Some(bin.join("xagent"));
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
        let mut cmd = std::process::Command::new(bin.join("agent-session"));
        cmd.args(args).current_dir(&deep).env_clear().env("PATH", "/usr/bin:/bin")
            .env(buckyos_api::ENV_EXECUTOR_CALL_ID, "call-7")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.unwrap_or_default().as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let soon = libopendan::now_ms() + 30 * 60 * 1000;
    write_context(&key.sign(loader.agent.agent_id(), sd.sid(), soon));

    let out = helper(&["sessions"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(sd.sid()));
    let out = helper(&["todo", "add", "from the node", "--content", "-"], Some("the Todo's text"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let list = libopendan::todos::store_of(&sd).load().unwrap().render_view();
    assert!(list.contains("from the node"), "{list}");
    // The token was close to its end: it was renewed on the way.
    let renewed = std::fs::read_to_string(workdir.join(CONTEXT_DIR).join(TOKEN_FILE)).unwrap();
    assert!(libopendan::session_context::token_expiry(&renewed).unwrap() > soon);

    // Tasks of a run that is not in progress here; ask-user answers for the
    // call it belongs to without asking the service.
    let out = helper(&["task", "state", "t-1"], None);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stdout));
    let out = helper(&["ask-user", "Which database?"], None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let asked: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(asked["task_id"], "userinput:call-7");

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
