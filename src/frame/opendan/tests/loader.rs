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
    AiMessage, AiResponse, AiRole, AiUsage, MailboxAddress, MailboxKind, MailboxRecord,
    MailboxRecordWithObject, MsgEditCapability, PostSendResult, RecipientState, TaskWaitReason,
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
use opendan::loader::{Loader, LoaderEnv};
use opendan::service::{Access, SERVICE_PATH};
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
    script: Box<dyn Fn(&LlmInferenceRequest, usize) -> String + Send + Sync>,
}

impl Llm {
    fn new(f: impl Fn(&LlmInferenceRequest, usize) -> String + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
            script: Box::new(f),
        })
    }
}

#[async_trait]
impl LlmClient for Llm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        if self.failures.load(Ordering::SeqCst) > 0 {
            self.failures.fetch_sub(1, Ordering::SeqCst);
            return Err(LLMComputeError::Provider {
                failure: ProviderFailure::Unknown,
                message: "provider refused the request".into(),
            });
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        // `TOOL:<command>` is a shell call instead of an answer.
        let answer = (self.script)(&req, n);
        let message = match answer.strip_prefix("TOOL:") {
            Some(command) => AiMessage::new(
                AiRole::Assistant,
                vec![buckyos_api::AiContent::tool_use(
                    format!("c{n}"),
                    "shell",
                    [("command".to_string(), json!(command))].into_iter().collect(),
                )],
            ),
            None => AiMessage::text(AiRole::Assistant, answer),
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
    /// The conversations can replace a message in place.
    editable: AtomicBool,
    seq: AtomicUsize,
}

impl Mail {
    /// A message arriving in the agent's inbox `session`.
    fn deliver(&self, session: &str, msg: MsgObject) -> String {
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
            from_name: Some("Bob".into()),
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

    fn env(&self, llm: Arc<dyn LlmClient>, port: u16) -> LoaderEnv {
        LoaderEnv {
            who: WHO.into(),
            agent_did: AGENT.into(),
            agent_id: None,
            agent_root: self.root.clone(),
            package_root: None,
            channels: self.channels(),
            waker: Arc::new(PollWaker),
            kevent: None,
            mail: Some(self.mail.clone()),
            tasks: self.tasks.clone().map(|t| t as Arc<dyn TaskService>),
            access: Access::Open { who: WHO.into() },
            port,
            web_dir: None,
            xllm: XllmDeps::default().with_llm(llm),
            session_cli: None,
            options: RunnerOptions {
                poll_interval: Duration::from_millis(50),
                max_wait: Duration::from_millis(300),
                load_hints: false,
                ..Default::default()
            },
        }
    }

    async fn start(&self, llm: Arc<dyn LlmClient>) -> Loader {
        Loader::start(self.env(llm, free_port())).await.unwrap()
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
    let loader = Loader::start(w.env(llm, port)).await.unwrap();
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
    let cursor = remote.perception().cursor().await.unwrap();
    assert_eq!(cursor, local.perception().cursor().await.unwrap());
    assert_eq!(
        remote.perception().backlog(&cursor).await.unwrap(),
        local.perception().backlog(&cursor).await.unwrap()
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
    assert!(remote.locks().is_held("self_improve").is_err());

    // Observation and the Loader's own state.
    let view = rpc(port, "session.read", json!({ "sid": ui.sid() })).await.unwrap();
    assert_eq!(view["entry"]["session_id"], ui.sid());
    assert_eq!(view["state"]["turns_completed"], 1);
    assert_eq!(view["config"]["session"]["class"], "ui");
    assert!(view["worklog"].as_array().is_some_and(|w| !w.is_empty()));
    assert_eq!(view["hosted"]["session_id"], ui.sid());
    let status = rpc(port, "loader.status", json!({})).await.unwrap();
    assert_eq!(status["agent_did"], AGENT);
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

    // The home page: profile card, usage by model, conversations.
    let profile = rpc(port, "agent.profile", json!({})).await.unwrap();
    assert_eq!(profile["agent_did"], AGENT);
    assert!(profile["owner_did"].is_null() && profile["desktop_url"].is_null());
    let profile = rpc(port, "agent.profile_set", json!({ "display_name": "Jay", "bio": "at your service" }))
        .await
        .unwrap();
    assert_eq!(profile["display_name"], "Jay");
    assert_eq!(rpc(port, "agent.profile", json!({})).await.unwrap()["bio"], "at your service");
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

    // A group conversation is another inbox, another session, and the
    // reply goes to the group; the agent's own words coming back are noise.
    let group = "did:bns:dev-team";
    let mut gmsg = chat(BOB, "@jarvis hello from the group");
    gmsg.kind = MsgObjKind::GroupMsg;
    gmsg.to = vec![parse_did(group).unwrap()];
    w.mail.deliver(group, gmsg);
    until("group reply", || w.mail.sent().len() == 3).await;
    let (_, greply) = w.mail.sent().remove(2);
    assert_eq!(greply.to, vec![parse_did(group).unwrap()]);
    assert_eq!(greply.kind, MsgObjKind::GroupMsg);
    let groute = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(group.to_string()))
        .unwrap()
        .to_string();
    let gentry = ui_entry(loader.agent.as_ref(), &groute).unwrap();
    assert_ne!(gentry.session_id, entry.session_id);
    assert_eq!(gentry.class, "group");
    let mut echo = greply.clone();
    echo.created_at_ms += 1;
    w.mail.deliver(group, echo);
    until("echo acknowledged", || w.mail.unread() == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(w.mail.sent().len(), 3);

    // `/stop` from the owner ends the session; the next message of that
    // inbox opens the next generation.
    w.mail.deliver(&session, chat("did:bns:alice", "/stop"));
    until("stopped", || sd.state().map(|s| s.is_finished()).unwrap_or(false)).await;
    w.mail.deliver(&session, chat(BOB, "hello once more"));
    until("reply of the next generation", || w.mail.sent().len() == 4).await;
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
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/jarvis_runtime/agent");
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
        "你好".to_string()
    });
    let mut env = w.env(llm, free_port());
    env.package_root = Some(package);
    let loader = Loader::start(env).await.unwrap();
    assert_eq!(loader.config.language, "zh");
    assert_eq!(loader.config.llm_context["provider"]["type"], "buckyos");
    for name in ["chat_route", "task_route", "plan", "do"] {
        let b = loader.agent.behaviors().get(name).await.unwrap().unwrap();
        assert!(b.prompt.system.is_some(), "{name}");
    }
    let session = format!("dm:{BOB}");
    w.mail.deliver(&session, chat(BOB, "你好 Jarvis"));
    until("reply", || w.mail.sent().len() == 1).await;
    let (_, reply) = w.mail.sent().remove(0);
    assert_eq!(reply.content.content, "你好");
    assert!(reply.meta["delivery_failure_notice"].as_str().unwrap().contains("没能送达"));
    let system = seen.lock().unwrap().clone();
    assert!(system.contains("You are Jarvis"), "identity is part of the system text");
    assert!(system.contains("one-to-one UI session"), "{system}");
    assert!(!system.contains("<<"), "{system}");
    let tools = tools.lock().unwrap().clone();
    assert!(tools.iter().any(|t| t == "shell") && tools.iter().any(|t| t == "call_behavior"), "{tools:?}");
    assert_eq!(cfg_max_tokens(&loader.config.llm_context), Some(8192));
    let route = MailboxAddress::new(parse_did(AGENT).unwrap(), Some(session))
        .unwrap()
        .to_string();
    let entry = ui_entry(loader.agent.as_ref(), &route).unwrap();
    let cfg = SessionDir::open(&entry.location).unwrap().config().unwrap();
    assert_eq!(cfg.prompt.behavior.as_deref(), Some("chat_route"));
    let frozen = cfg.prompt.frozen.unwrap();
    assert!(frozen.behaviors.contains_key("chat_route") && frozen.behaviors.contains_key("task_route"));
    // A work session of the package starts in plan and can hand over to do.
    let template = SessionTemplate::load("work", loader.agent.agent_root()).unwrap();
    assert_eq!(template.default_behavior.as_deref(), Some("plan"));
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
    let loader = w.start(llm).await;
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
    loader.shutdown().await;
}
