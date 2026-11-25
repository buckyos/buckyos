//! What an exploration can do on a runtime whose grant only reads
//! (`AgentPermissionMode::ReadOnly`), through a real executor running as the
//! current user: the Jarvis `explore` behavior, a scripted model.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agent_tool::runtime::node::{NodeExecutorConnector, NodeIo};
use agent_tool::xllm::XllmDeps;
use async_trait::async_trait;
use buckyos_api::*;
use libopendan::channel::KmsgChannels;
use libopendan::protocol::*;
use libopendan::runner::{drive, RunnerDeps, RunnerOptions, StopWhen};
use libopendan::state::AgentStateClient;
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::LLMComputeError;
use node_executor::{ExecutorAdmission, ExecutorHost, ExecutorIdentityMode, ExecutorOptions, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher};
use serde_json::{json, Value};

const APP: &str = "jarvis.test.buckyos.io";
const WHO: &str = "app:jarvis@devtest";
const NODE_DID: &str = "did:web:ood1.test.buckyos.io";
const PACKAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/jarvis_agent/agent");

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String> {
        let inst: AppInstanceId = token.parse().map_err(|_| "bad")?;
        Ok(VerifiedSession {
            subject: inst.owner_user_id().into(),
            principal_kind: TokenPrincipalKind::App,
            token_use: TokenUse::Session,
            authorization_key: format!("app:{}", inst.app_id()),
            app_instance_id: Some(inst),
            expires_at_ms: node_executor::now_ms() + 600_000,
        })
    }
}

struct AllowAll;
#[async_trait]
impl RbacChecker for AllowAll {
    async fn allowed(&self, _: &str, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

struct InProcess {
    exec: NodeExecutor,
}

#[async_trait]
impl NodeExecutorConnector for InProcess {
    async fn connect(&self, _: &str) -> std::io::Result<Box<dyn NodeIo>> {
        let (a, b) = tokio::io::duplex(1 << 20);
        let exec = self.exec.clone();
        tokio::spawn(async move { exec.serve(b).await });
        Ok(Box::new(a))
    }
    async fn session_token(&self) -> Result<String, String> {
        Ok(format!("{APP}@devtest"))
    }
}

/// Makes its calls one by one, keeps what each returned, then reports.
struct Calls {
    calls: Vec<(&'static str, Value)>,
    results: Mutex<BTreeMap<String, String>>,
}

fn tool_call(id: &str, name: &str, args: Value) -> AiResponse {
    AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        vec![AiContent::tool_use(id, name, args.as_object().unwrap().clone().into_iter().collect())],
    ))
}

#[async_trait]
impl LlmClient for Calls {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let mut done = self.results.lock().unwrap();
        for m in &req.messages {
            for c in &m.content {
                if let AiContent::ToolResult { call_id, content, .. } = c {
                    done.insert(call_id.clone(), format!("{content:?}"));
                }
            }
        }
        for (i, (name, args)) in self.calls.iter().enumerate() {
            let id = format!("c{i}");
            if !done.contains_key(&id) {
                return Ok(tool_call(&id, name, args.clone()));
            }
        }
        Ok(tool_call("end", "report", json!({ "report": "read what could be read", "outcome": "succeeded", "is_end": true })))
    }
}

fn copy(name: &str, to: &Path) {
    let target = to.join(name);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::copy(Path::new(PACKAGE).join(name), target).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exploration_reads_on_a_read_only_runtime() {
    let dir = tempfile::Builder::new().prefix("nx-ro").tempdir_in("/tmp").unwrap();
    let host = dir.path();
    let work = host.join("work");
    std::fs::create_dir_all(work.join("logs")).unwrap();
    std::fs::write(work.join("notes.txt"), "the port is 9100\n").unwrap();
    std::fs::write(work.join("logs/app.log"), "ERROR disk full\n").unwrap();
    let worker = host.join("node_executor_worker");
    std::fs::copy(env!("CARGO_BIN_EXE_node_executor_worker"), &worker).unwrap();
    let exec = NodeExecutor::open_with_options(
        ExecutorHost {
            node_id: "ood1".into(),
            node_did: NODE_DID.into(),
            store_dir: host.join("ledger"),
            worker: WorkerLauncher { program: worker, args: vec![] },
            shell: node_executor::default_shell(),
            verifier: Arc::new(Verifier),
            rbac: Arc::new(AllowAll),
        },
        ExecutorOptions {
            identity_mode: ExecutorIdentityMode::CurrentUser,
            admission: ExecutorAdmission::Node,
            path_prefix: Vec::new(),
            buckyos_root: None,
        },
    )
    .unwrap();
    let agent_did = AppId::parse(APP).unwrap().app_did().to_string();
    exec.apply_config(Some(ExecutorConfig {
        enabled: true,
        revision: "sha256:r1".into(),
        generation: 1,
        auth_valid_until_ms: node_executor::now_ms() + 60_000,
        allowed_devices: vec!["did:dev:peer".into()],
        principal_bindings: BTreeMap::from([(
            agent_did.clone(),
            ExecutorPrincipalBinding {
                app_instance_id: format!("{APP}@devtest").parse().unwrap(),
                agent_generation: 1,
                binding_generation: 1,
                enabled: true,
                app_installed: true,
                permission_mode: AgentPermissionMode::ReadOnly,
            },
        )]),
        profiles: BTreeMap::from([(
            "dev".into(),
            ExecutorProfile {
                baseline: None,
                run_as: None,
                workdir_roots: vec![work.to_string_lossy().to_string()],
                env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                principals: vec![agent_did.clone()],
                ops: ExecutorOp::ALL.to_vec(),
            },
        )]),
        limits: ExecutorLimits::default(),
    }))
    .await;
    libopendan::runtime::set_node_connector(Arc::new(InProcess { exec: exec.clone() }));

    let agent_root = host.join("agent_root");
    for f in ["agent.toml", "role.md", "self.md", "behaviors/explore.toml", "behaviors/work_simple.toml"] {
        copy(f, &agent_root);
    }
    let channels = Arc::new(KmsgChannels::dir(&host.join("kmsg")).unwrap());
    let agent = Arc::new(
        libopendan::FsAgentStateClient::open(&agent_root, &agent_did, Some(channels.client()), None).unwrap(),
    );
    let record = agent
        .runtimes()
        .register(
            &RuntimeRegister {
                runtime_id: "box".into(),
                connector: RuntimeConnector::NodeExecutor { node_did: Some(NODE_DID.into()), executor_id: None, profile_id: "dev".into() },
                name: "Box".into(),
                desc: String::new(),
                enabled: true,
                default: false,
                baseline: RuntimeBaseline::Standard,
                grant: RuntimeGrant { roots: vec![work.clone()], ..Default::default() },
            },
            "owner",
        )
        .await
        .unwrap();
    assert!(record.usable() && record.grant.read_only() && !record.grant.can_exec(), "{record:?}");

    let sessions = host.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let spec = |behavior: &str| {
        let mut s = libopendan::SessionTemplate::load("work", Some(&agent_root)).unwrap().spec("look at the box");
        s.prompt.behavior = Some(behavior.into());
        s.prompt.llm_context = json!({ "tools": { "enabled": true } });
        s.runtime.requirement.runtime_id = Some("box".into());
        s.idempotency_key = Some(behavior.into());
        s.freeze = true;
        s
    };
    let refused = libopendan::api::create_session(&sessions, spec("work_simple"), agent.as_ref(), WHO, channels.as_ref()).await.unwrap_err();
    assert!(refused.to_string().contains("grants this agent read-only operations"), "{refused}");

    let sd = libopendan::api::create_session(&sessions, spec("explore"), agent.as_ref(), WHO, channels.as_ref()).await.unwrap();
    let path = |p: &str| work.join(p).display().to_string();
    let llm = Arc::new(Calls {
        calls: vec![
            ("read_file", json!({ "path": path("notes.txt") })),
            ("read_file", json!({ "path": path("logs") })),
            ("shell", json!({ "command": format!("ls {}", work.display()) })),
            ("write_file", json!({ "path": path("probe.txt"), "content": "x" })),
            ("read_file", json!({ "path": "logs/app.log" })),
            ("read", json!({ "uri": path("logs/app.log") })),
            ("edit_file", json!({ "path": path("notes.txt"), "old_string": "9100", "new_string": "1" })),
        ],
        results: Mutex::default(),
    });
    let cfg = sd.config().unwrap();
    let xllm = XllmDeps::default().with_llm(llm.clone());
    let runtime = libopendan::runtime::runtime_for_session(&sd, &cfg, agent.as_ref(), None, &xllm).await.unwrap();
    let deps = RunnerDeps::new(WHO, agent.clone(), channels, runtime, xllm).with_options(RunnerOptions {
        load_hints: false,
        ..Default::default()
    });
    let r = Box::pin(drive(&sd, &deps, StopWhen::Finished)).await;
    let results = llm.results.lock().unwrap().clone();
    for (id, text) in &results {
        println!("{id}: {}", text.chars().take(400).collect::<String>());
    }
    assert!(r.is_finished(), "{r:?} {:?}", sd.state().unwrap().last_error);
    assert_eq!(sd.binding_opt().unwrap().unwrap().workdir, work.canonicalize().unwrap().display().to_string());
    assert!(results["c0"].contains("the port is 9100"), "it reads the files there");
    assert!(results["c4"].contains("disk full"), "paths start from the directory it works in");
    for id in ["c2", "c3", "c6"] {
        assert!(!results[id].contains("disk full") && (results[id].contains("rror") || results[id].contains("not ") || results[id].contains("denied")), "{id}: {}", results[id]);
    }
    assert!(!work.join("probe.txt").exists() && std::fs::read_to_string(work.join("notes.txt")).unwrap().contains("9100"));
    let entries: Vec<_> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(entries.len(), 2, "nothing was written: {entries:?}");
}
