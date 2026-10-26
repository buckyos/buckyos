//! Runtime registrations (intent analysis TODO §3.12, V01–V04).

mod common;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use libopendan::api::{create_sub_session, SubSessionSpec};
use libopendan::protocol::*;
use libopendan::state::{AgentStateClient, FsAgentStateClient, KrpcAgentStateClient, StateTransport};
use libopendan::{OpenDanError, Result};
use serde_json::{json, Value};

fn state(path: &Path) -> FsAgentStateClient {
    FsAgentStateClient::open(path.join("agent"), "did:test:agent", None, None).unwrap()
}

fn builtin(path: &Path) -> Vec<RuntimeRegister> {
    builtin_runtimes(vec![path.join("workspaces")], "", &Default::default(), "did:test:agent", false)
}

/// V01 / V02: the host creates the built-in runtimes once; what the owner
/// changes of them stays; they cannot be unregistered; added runtimes are
/// CAS-updated and removable.
#[tokio::test]
async fn builtin_runtimes_are_created_once_and_kept() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let r = state.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    assert_eq!(r.iter().map(|r| r.runtime_id.as_str()).collect::<Vec<_>>(), ["container", "host"]);
    let container = &r[0];
    assert_eq!((container.availability, container.builtin), (RuntimeAvailability::Available, true));
    assert_eq!(container.exec, Some(json!({ "kind": "native" })));
    assert_eq!(container.registered_by, "host");
    let host = &r[1];
    assert_eq!(host.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(host.exec.is_none() && host.last_error.as_deref().unwrap().contains("not enabled"));
    assert!(r[0].default && !host.default);

    // The owner renames the container; a restart keeps it and adds nothing.
    let renamed = state
        .runtimes()
        .update(
            "container",
            &RuntimeUpdate {
                expected_revision: container.revision,
                name: Some("My box".into()),
                ..Default::default()
            },
            "owner",
        )
        .await
        .unwrap();
    assert_eq!(renamed.revision, container.revision + 1);
    let again = state.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    assert_eq!(again[0].name, "My box");
    assert_eq!(again[0].revision, renamed.revision, "nothing changed");
    assert_eq!(state.runtimes().list().await.unwrap().len(), 2);
    // `local` is the container's former name.
    assert_eq!(state.runtimes().lookup("local").await.unwrap().unwrap().name, "My box");

    let stale = state
        .runtimes()
        .update("container", &RuntimeUpdate { expected_revision: 1, ..Default::default() }, "owner")
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("revision_conflict"), "{stale}");
    let err = state
        .runtimes()
        .unregister("host", host.revision, "owner")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("built in"), "{err}");

    let added = RuntimeRegister {
        runtime_id: "studio".into(),
        kind: RuntimeKind::RemoteSsh,
        name: "Studio workstation".into(),
        description: String::new(),
        exec: Some(json!({ "kind": "remote_ssh", "host": "studio" })),
        roots: vec![],
        capabilities: json!({}),
        default: false,
        computer: Default::default(),
        baseline: buckyos_api::RuntimeBaseline::Standard,
    };
    let studio = state.runtimes().register(&added, "owner").await.unwrap();
    assert_eq!(studio, state.runtimes().register(&added, "owner").await.unwrap(), "idempotent");
    assert!(!studio.builtin);
    assert_eq!(studio.availability, RuntimeAvailability::RuntimeUnavailable);
    let mut other = added.clone();
    other.name = "else".into();
    assert!(state.runtimes().register(&other, "owner").await.is_err());
    assert!(state.runtimes().register(&RuntimeRegister { runtime_id: "local".into(), ..added.clone() }, "owner").await.is_err());
    state.runtimes().unregister("studio", studio.revision, "owner").await.unwrap();
    assert!(state.runtimes().lookup("studio").await.unwrap().is_none());
}

struct Loopback(Arc<FsAgentStateClient>);

#[async_trait]
impl StateTransport for Loopback {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        libopendan::state::krpc::serve_call(self.0.as_ref(), "caller", method, &params)
            .await
            .unwrap_or_else(|| Err(OpenDanError::NotFound(method.into())))
    }
}

/// V01: the registry over kRPC signs writes with the server's identity;
/// only the host registers built-in runtimes.
#[tokio::test]
async fn runtimes_over_krpc() {
    let temp = tempfile::tempdir().unwrap();
    let fs = Arc::new(state(temp.path()));
    fs.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    let client = KrpcAgentStateClient::new("did:test:agent", Arc::new(Loopback(fs)));
    let all = client.runtimes().list().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(client.runtimes().lookup("host").await.unwrap().unwrap(), all[1]);
    let checked = client.runtimes().check("container").await.unwrap();
    assert_eq!(checked.availability, RuntimeAvailability::Available);
    let added = client
        .runtimes()
        .register(
            &RuntimeRegister {
                runtime_id: "lab".into(),
                kind: RuntimeKind::HostNative,
                name: "Lab".into(),
                description: String::new(),
                exec: None,
                roots: vec![],
                capabilities: json!({}),
                default: false,
                computer: Default::default(),
                baseline: buckyos_api::RuntimeBaseline::Standard,
            },
            "forged",
        )
        .await
        .unwrap();
    assert_eq!(added.registered_by, "caller");
    assert!(client.runtimes().ensure_builtin(&builtin(temp.path()), "x").await.is_err());
    for m in ["register", "update", "check", "unregister"] {
        assert!(libopendan::state::krpc::is_write(&format!("runtimes.{m}")));
    }
    assert!(!libopendan::state::krpc::is_write("runtimes.list"));
}

/// V04: workspaces live in the container runtime (`local` still read);
/// the host runtime cannot hold one yet and says why.
#[tokio::test]
async fn workspaces_follow_the_runtime_registry() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    state.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    let create = |runtime: &str, dir: &str, op: &str| WorkspaceCreate {
        operation_id: op.into(),
        name: "P".into(),
        description: String::new(),
        url: None,
        location: WorkspaceLocation {
            runtime_id: runtime.into(),
            directory: temp.path().join(dir),
        },
        usage: WorkspaceUsage::Collaborative,
        source_session: None,
        policy_ref: None,
    };
    let a = state.workspaces().create(&create("container", "a", "a"), "u").await.unwrap();
    assert_eq!(a.location.runtime_id, "container");
    let b = state.workspaces().create(&create("local", "b", "b"), "u").await.unwrap();
    assert_eq!(b.location.runtime_id, "local");
    // One runtime: overlapping directories are refused across both names.
    let nested = state.workspaces().create(&create("container", "b/inner", "c"), "u").await.unwrap_err();
    assert!(nested.to_string().contains("overlapping_directory"), "{nested}");
    let err = state.workspaces().create(&create("host", "h", "h"), "u").await.unwrap_err();
    assert!(err.to_string().contains("runtime_unavailable") && err.to_string().contains("host"), "{err}");
    assert_eq!(
        state
            .workspaces()
            .query(&WorkspaceQuery { runtime_id: Some("container".into()), ..Default::default() })
            .await
            .unwrap()
            .len(),
        2
    );
}

/// V03: a Work asked to run in a runtime that is not registered or not
/// usable is not created.
#[tokio::test]
async fn a_work_session_needs_a_usable_registered_runtime() {
    let env = Env::new();
    let agent = env.agent();
    agent
        .runtimes()
        .ensure_builtin(&builtin(&env.root), APP)
        .await
        .unwrap();
    let parent = env.create_work(work_spec("parent")).await;
    let sub = |runtime: &str| SubSessionSpec {
        objective: "child".into(),
        runtime_id: Some(runtime.into()),
        key: Some(format!("k-{runtime}")),
        ..Default::default()
    };
    let ch = env.channels();
    for (runtime, msg) in [("host", "not available"), ("nowhere", "not registered")] {
        let err = create_sub_session(agent.as_ref(), APP, parent.sid(), sub(runtime), ch.as_ref())
            .await
            .unwrap_err();
        assert!(matches!(err, OpenDanError::Bind(_)), "{err:?}");
        assert!(err.to_string().contains(msg), "{err}");
    }
    let sd = create_sub_session(agent.as_ref(), APP, parent.sid(), sub("container"), ch.as_ref())
        .await
        .unwrap();
    assert!(sd.config().unwrap().runtime.requirement.runtime_id.is_none());
}

/// A node executor that refuses connections until it is `up`, then answers
/// login and describe (what a runtime check asks).
struct LateNode {
    up: Arc<std::sync::atomic::AtomicBool>,
    connects: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl agent_tool::runtime::node::NodeExecutorConnector for LateNode {
    async fn connect(&self, _node: &str) -> std::io::Result<Box<dyn agent_tool::runtime::node::NodeIo>> {
        use std::sync::atomic::Ordering;
        use tokio::io::AsyncWriteExt;
        self.connects.fetch_add(1, Ordering::SeqCst);
        if !self.up.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "early eof"));
        }
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
                        "principal":"did:test:agent", "app_instance_id":"app2@alice", "caps":["read","stat","exec"],
                        "limits":buckyos_api::ExecutorLimits::default(), "revision":"one", "generation":1,
                        "auth_valid_until_ms":u64::MAX, "token_expires_at_ms":u64::MAX,
                        "store_id":"late-ledger", "server_epoch":"one"})
                    }
                    "describe" => json!({"type":"describe_ok", "request_id":id,
                        "target":{"node_did":"did:web:ood1.test.buckyos.io", "profile_id":profile, "uid":65534, "gid":65534,
                            "workdir":request["workdir"], "workdir_roots":["/srv/host-runtime"], "store_id":"late-ledger", "protocol_version":1},
                        "info":{"os":"linux", "arch":"x86_64", "hostname":"ood1", "shell":"bash", "time_ms":1, "identity_mode":"switch_user"}}),
                    other => panic!("unexpected executor operation: {other}"),
                };
                server.write_all(&buckyos_api::encode_executor_control(&response).unwrap()).await.unwrap();
            }
        });
        Ok(Box::new(client))
    }

    async fn session_token(&self) -> std::result::Result<String, String> {
        Ok("late-session".into())
    }
}

/// A node runtime whose executor was not up yet when the agent checked it
/// (node_daemon opens it after the agent's container starts) is asked again
/// by the next session that needs it, at most every RUNTIME_RECHECK_MS:
/// sessions are not refused forever after a transient failure.
#[tokio::test]
async fn a_node_runtime_refused_at_start_is_used_once_its_executor_answers() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let env = Env::new();
    let agent = env.agent();
    let up = Arc::new(AtomicBool::new(false));
    let connects = Arc::new(AtomicUsize::new(0));
    libopendan::runtime::set_node_connector(Arc::new(LateNode { up: up.clone(), connects: connects.clone() }));
    let mut settings = buckyos_api::AgentSettings::default();
    settings.runtimes.insert(
        "host".into(),
        buckyos_api::AgentRuntimeSettings {
            enabled: true,
            computer: Default::default(),
            target: buckyos_api::AgentRuntimeTarget::Node {
                node_id: "ood1".into(),
                node_did: "did:web:ood1.test.buckyos.io".into(),
            },
            baseline: None,
        },
    );
    settings.default_runtime_id = Some("host".into());
    let registers = builtin_runtimes(vec![env.root.join("workspaces")], "", &settings, AGENT, false);
    agent.runtimes().ensure_builtin(&registers, APP).await.unwrap();
    let checked = agent.runtimes().check("host").await.unwrap();
    assert_eq!(checked.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(checked.last_error.unwrap_or_default().contains("early eof"));
    let tries = connects.load(Ordering::SeqCst);
    // Within the recheck interval the earlier answer stands.
    let spec = || {
        let mut s = work_spec("on the host");
        s.runtime.requirement.runtime_id = None;
        s
    };
    let refused = libopendan::api::create_session(&env.app_dir, spec(), agent.as_ref(), APP, env.channels().as_ref())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("not available"), "{refused}");
    assert_eq!(connects.load(Ordering::SeqCst), tries, "not asked again right away");
    // The executor comes up; after the interval the next session asks again.
    up.store(true, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(libopendan::api::RUNTIME_RECHECK_MS + 200)).await;
    let sd = env.create_work(spec()).await;
    let frozen = sd.config().unwrap().runtime.frozen.unwrap();
    assert_eq!(frozen.runtime_id, "host");
    let record = agent.runtimes().lookup("host").await.unwrap().unwrap();
    assert_eq!(record.availability, RuntimeAvailability::Available);
    assert!(connects.load(Ordering::SeqCst) > tries);
}

/// An Agent Service run natively on its host has no container: the
/// `container` runtime has no executor (with the reason), `host` is the
/// default; back in its container, `container` is usable again.
#[tokio::test]
async fn a_natively_run_agent_service_has_no_container_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let mut settings = buckyos_api::AgentSettings::default();
    settings.runtimes.insert(
        "host".into(),
        buckyos_api::AgentRuntimeSettings {
            enabled: true,
            computer: Default::default(),
            target: buckyos_api::AgentRuntimeTarget::Node { node_id: "ood1".into(), node_did: "did:web:ood1.test.buckyos.io".into() },
            baseline: None,
        },
    );
    settings.default_runtime_id = Some("host".into());
    let project = |native: bool| builtin_runtimes(vec![temp.path().join("workspaces")], "", &settings, "did:test:agent", native);
    state.runtimes().ensure_builtin(&project(false), "host").await.unwrap();
    let container = state.runtimes().lookup("container").await.unwrap().unwrap();
    assert_eq!(container.availability, RuntimeAvailability::Available);
    state.runtimes().ensure_builtin(&project(true), "host").await.unwrap();
    let container = state.runtimes().lookup("container").await.unwrap().unwrap();
    assert_eq!(container.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(container.exec.is_none() && !container.default);
    assert!(container.last_error.unwrap_or_default().contains("runs natively"));
    assert!(state.runtimes().lookup("host").await.unwrap().unwrap().default);
    state.runtimes().ensure_builtin(&project(false), "host").await.unwrap();
    let container = state.runtimes().lookup("container").await.unwrap().unwrap();
    assert_eq!(container.availability, RuntimeAvailability::Available);
}
