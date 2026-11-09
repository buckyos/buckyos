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

fn builtin(_path: &Path) -> Vec<RuntimeRegister> {
    builtin_runtimes(&Default::default(), "did:test:agent", &PrivateRuntimeInfo::default())
}

/// V01 / V02: the host creates the built-in runtimes once; what the owner
/// changes of them stays; they cannot be unregistered; added runtimes are
/// CAS-updated and removable.
#[tokio::test]
async fn builtin_runtimes_are_created_once_and_kept() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let r = state.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    assert_eq!(r.iter().map(|r| r.runtime_id.as_str()).collect::<Vec<_>>(), ["private", "host"]);
    let container = &r[0];
    assert_eq!((container.availability, container.builtin), (RuntimeAvailability::Available, true));
    assert_eq!(container.exec(), Some(json!({ "kind": "native" })));
    assert!(container.connector.is_native() && container.grant.workspace && !container.grant.read_only());
    assert_eq!(container.registered_by, "host");
    let host = &r[1];
    assert_eq!(host.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(host.exec().is_none() && host.last_error.as_deref().unwrap().contains("not enabled"));
    assert!(r[0].default && !host.default);

    // The owner renames the private runtime; a restart keeps it and adds nothing.
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
    // `local` and `container` are the private runtime's former names.
    assert_eq!(state.runtimes().lookup("local").await.unwrap().unwrap().name, "My box");
    assert_eq!(state.runtimes().lookup("container").await.unwrap().unwrap().runtime_id, "private");

    let stale = state
        .runtimes()
        .update("private", &RuntimeUpdate { expected_revision: 1, ..Default::default() }, "owner")
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
        connector: RuntimeConnector::Ssh { host: "studio".into(), user: None, port: None, identity_file: None },
        name: "Studio workstation".into(),
        desc: String::new(),
        enabled: true,
        default: false,
        baseline: buckyos_api::RuntimeBaseline::Standard,
        grant: RuntimeGrant { roots: vec!["/srv/studio".into()], ..Default::default() },
    };
    // A registration without a root is refused (R-K5).
    let rootless = RuntimeRegister { grant: RuntimeGrant::default(), ..added.clone() };
    assert!(state.runtimes().register(&rootless, "owner").await.unwrap_err().to_string().contains("root"));
    let studio = state.runtimes().register(&added, "owner").await.unwrap();
    assert_eq!(studio, state.runtimes().register(&added, "owner").await.unwrap(), "idempotent");
    assert!(!studio.builtin);
    assert_eq!(studio.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(studio.checked_at_ms.is_some(), "checked at once");
    let mut other = added.clone();
    other.name = "else".into();
    assert!(state.runtimes().register(&other, "owner").await.is_err());
    assert!(state.runtimes().register(&RuntimeRegister { runtime_id: "local".into(), ..added.clone() }, "owner").await.is_err());
    // Unregistering retires the record: it stays with its history.
    let retired = state.runtimes().unregister("studio", studio.revision, "owner").await.unwrap();
    assert_eq!(retired.lifecycle, RuntimeLifecycle::Retired);
    assert!(!retired.usable());
    let kept = state.runtimes().lookup("studio").await.unwrap().unwrap();
    assert_eq!(kept.lifecycle, RuntimeLifecycle::Retired);
    assert!(kept.last_error.as_deref().unwrap().contains("retired"));
}

/// The private runtime's check: the Agent's own commands come first, a named
/// command only reports its path until a full check runs it, and the
/// declaration stays what the host declared.
#[cfg(unix)]
#[tokio::test]
async fn the_private_runtime_says_who_provides_a_command() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    state.runtimes().ensure_builtin(&builtin(temp.path()), "host").await.unwrap();
    let skill_bin = temp.path().join("agent/tools/lint/bin");
    std::fs::create_dir_all(&skill_bin).unwrap();
    for (name, body) in [("real-tool", "echo 'real-tool 9.9'"), ("stub-tool", "echo 'not bundled' >&2; exit 127")] {
        let path = skill_bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let asked: Vec<String> = ["real-tool", "stub-tool", "sh", "no-such-tool-xyz"].iter().map(|c| c.to_string()).collect();

    let quick = state.runtimes().check("private", &asked, false).await.unwrap();
    let tools = quick.caps.tools_check.unwrap();
    assert_eq!(tools.present, ["real-tool", "stub-tool", "sh"], "{:?}", tools.error);
    assert_eq!(tools.missing, ["no-such-tool-xyz"]);
    assert_eq!(tools.resolved["real-tool"].provided_by, ProvidedBy::Agent);
    assert_eq!(tools.resolved["real-tool"].version, None);
    assert_eq!(tools.resolved["sh"].provided_by, ProvidedBy::System);
    let check = quick.caps.baseline_check.unwrap();
    assert_eq!(check.level, buckyos_api::RuntimeBaseline::Extended, "the container's private runtime is declared extended");
    assert!(check.ext.is_some() && check.error.is_none(), "{check:?}");

    let full = state.runtimes().check("private", &asked, true).await.unwrap();
    let tools = full.caps.tools_check.unwrap();
    assert_eq!(tools.present, ["real-tool", "sh"], "{:?}", tools.error);
    assert_eq!(tools.missing, ["stub-tool", "no-such-tool-xyz"], "a stub is not the tool");
    assert_eq!(tools.resolved["real-tool"].version.as_deref(), Some("real-tool 9.9"));
    assert!(tools.resolved["stub-tool"].stub);
    assert_eq!(full.caps.baseline, buckyos_api::RuntimeBaseline::Extended, "a check never changes the declaration");
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
    let checked = client.runtimes().check("container", &[], false).await.unwrap();
    assert_eq!((checked.runtime_id.as_str(), checked.availability), ("private", RuntimeAvailability::Available));
    mock_node(&["/lab"], "/lab/home");
    let added = client
        .runtimes()
        .register(
            &RuntimeRegister {
                runtime_id: "lab".into(),
                connector: RuntimeConnector::node("did:dev:lab", "did:test:agent"),
                name: "Lab".into(),
                desc: String::new(),
                enabled: true,
                default: false,
                baseline: buckyos_api::RuntimeBaseline::Standard,
                grant: RuntimeGrant { roots: vec!["/lab".into()], ..Default::default() },
            },
            "forged",
        )
        .await
        .unwrap();
    assert_eq!(added.registered_by, "caller");
    assert_eq!(added.availability, RuntimeAvailability::Available, "{:?}", added.last_error);
    assert!(added.grant.ops.contains(&buckyos_api::ExecutorOp::Exec), "ops learned at the check");
    assert_eq!(added.caps.platform.os, "linux");
    // `check --commands` verifies named commands (`tools_check`); the mock
    // node runs nothing, so the check says why.
    let verified = client.runtimes().check("lab", &["ffmpeg".into()], false).await.unwrap();
    assert!(verified.caps.tools_check.unwrap().error.is_some());
    client
        .runtimes()
        .note_usage("lab", &RuntimeUsageEvent::Session { session_id: "work-1".into() })
        .await
        .unwrap();
    let used = client.runtimes().lookup("lab").await.unwrap().unwrap();
    assert_eq!((used.usage.sessions_total, used.usage.last_session_id.as_deref()), (1, Some("work-1")));
    assert!(client.runtimes().ensure_builtin(&builtin(temp.path()), "x").await.is_err());
    for m in ["register", "update", "check", "unregister", "note_usage"] {
        assert!(libopendan::state::krpc::is_write(&format!("runtimes.{m}")));
    }
    assert!(!libopendan::state::krpc::is_write("runtimes.list"));
}

/// V04: workspaces live in the private runtime (`container` / `local` still
/// read); the host runtime cannot hold one yet and says why.
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
/// by the next session or command that needs it, at most every
/// RUNTIME_RECHECK_MS: nothing is refused forever after a transient failure.
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
            grant: None,
        },
    );
    settings.default_runtime_id = Some("host".into());
    let registers = builtin_runtimes(&settings, AGENT, &PrivateRuntimeInfo::default());
    agent.runtimes().ensure_builtin(&registers, APP).await.unwrap();
    let checked = agent.runtimes().check("host", &[], false).await.unwrap();
    assert_eq!(checked.availability, RuntimeAvailability::RuntimeUnavailable);
    assert!(checked.last_error.unwrap_or_default().contains("early eof"));
    let tries = connects.load(Ordering::SeqCst);
    // Within the recheck interval the earlier answer stands. The owner's
    // default never places work by itself: the work names the host.
    let spec = || {
        let mut s = work_spec("on the host");
        s.runtime.requirement.runtime_id = Some("host".into());
        s
    };
    let refused = libopendan::api::create_session(&env.app_dir, spec(), agent.as_ref(), APP, env.channels().as_ref())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("not available"), "{refused}");
    let refused = libopendan::runtime::usable_runtime(agent.as_ref(), "host").await.unwrap_err();
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

    // `agent-session run` and `copy` get the same second chance.
    up.store(false, Ordering::SeqCst);
    let checked = agent.runtimes().check("host", &[], false).await.unwrap();
    assert_eq!(checked.availability, RuntimeAvailability::RuntimeUnavailable);
    up.store(true, Ordering::SeqCst);
    let mut cfg = sd.config().unwrap();
    cfg.session.policy.runtime_view = RuntimeView::Chat;
    let tries = connects.load(Ordering::SeqCst);
    let view = libopendan::runtime::runtimes_view(agent.as_ref(), &cfg).await;
    assert_eq!(view["owner_default"]["available"], false);
    assert_eq!(connects.load(Ordering::SeqCst), tries);
    tokio::time::sleep(std::time::Duration::from_millis(libopendan::api::RUNTIME_RECHECK_MS + 200)).await;
    let view = libopendan::runtime::runtimes_view(agent.as_ref(), &cfg).await;
    assert_eq!(view["owner_default"]["available"], true);
    assert_eq!(view["owner_default"]["os"], "linux");
    assert!(view["owner_default"].get("error").is_none());
    let usable = libopendan::runtime::usable_runtime(agent.as_ref(), "host").await.unwrap();
    assert_eq!(usable.availability, RuntimeAvailability::Available);
}

/// An Agent Service run natively on its host (Desktop Lite) still has its
/// private runtime — the process's own environment, always usable — while
/// `host` stays the owner's default; back in its container nothing changes
/// for the private runtime but its description and baseline.
#[tokio::test]
async fn a_natively_run_agent_service_keeps_its_private_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let mut settings = buckyos_api::AgentSettings::default();
    settings.runtimes.insert(
        "host".into(),
        buckyos_api::AgentRuntimeSettings {
            enabled: true,
            computer: Default::default(),
            target: buckyos_api::AgentRuntimeTarget::Node { node_id: "ood1".into(), node_did: "did:web:ood1.test.buckyos.io".into() },
            baseline: Some(buckyos_api::RuntimeBaseline::Extended),
            grant: None,
        },
    );
    settings.default_runtime_id = Some("host".into());
    for native_node_did in [None, Some("did:web:another-node".into())] {
        let separate = builtin_runtimes(&settings, "did:test:agent", &PrivateRuntimeInfo {
            native_service: true, native_node_did, ..Default::default()
        });
        assert_eq!(separate.len(), 2);
        assert!(separate[1].connector.is_node());
        assert!(!separate[0].default);
    }
    let project = |native: bool| {
        builtin_runtimes(&settings, "did:test:agent", &PrivateRuntimeInfo { native_service: native, native_node_did: Some("did:web:ood1.test.buckyos.io".into()), note: String::new(), ..Default::default() })
    };
    state.runtimes().ensure_builtin(&project(false), "host").await.unwrap();
    let private = state.runtimes().lookup("private").await.unwrap().unwrap();
    assert_eq!(private.availability, RuntimeAvailability::Available);
    assert_eq!(private.caps.baseline, buckyos_api::RuntimeBaseline::Extended);
    state.runtimes().ensure_builtin(&project(true), "host").await.unwrap();
    let private = state.runtimes().lookup("private").await.unwrap().unwrap();
    assert_eq!(private.availability, RuntimeAvailability::Available, "{:?}", private.last_error);
    assert_eq!(private.exec(), Some(json!({ "kind": "native" })));
    assert!(private.default && private.desc.contains("natively"));
    assert_eq!(state.runtimes().list().await.unwrap().len(), 1);
    assert_eq!(private.caps.baseline, buckyos_api::RuntimeBaseline::Extended, "the host's declaration");
    let host = state.runtimes().lookup("host").await.unwrap().unwrap();
    assert!(host.default && host.grant.workspace);
    assert_eq!(host.runtime_id, "private");
    assert_eq!(host, private);
    state.runtimes().ensure_builtin(&project(false), "host").await.unwrap();
    let private = state.runtimes().lookup("private").await.unwrap().unwrap();
    assert!(private.availability == RuntimeAvailability::Available && !private.desc.contains("natively"));
}

#[cfg(unix)]
#[tokio::test]
async fn native_alias_checks_and_executes_managed_tools_without_an_executor() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let old_root = std::env::var_os("BUCKYOS_ROOT");
    std::env::set_var("BUCKYOS_ROOT", &root);
    let tool_dir = root.join("bin/agent-tools");
    std::fs::create_dir_all(&tool_dir).unwrap();
    let tool = tool_dir.join("native-regression-tool");
    std::fs::write(&tool, "#!/bin/sh\nprintf native-managed-tool").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = state(&root);
    let mut builtins = builtin(&root);
    builtins.truncate(1);
    builtins[0].default = true;
    builtins[0].connector = RuntimeConnector::Native {
        aliases: vec!["host".into()],
        computer: Some(agent_tool::runtime::computer::NativeComputerConfig {
            endpoint: root.join("missing.sock").display().to_string(),
            node_did: "did:web:test".into(),
            profile_id: "agent-test".into(), access: buckyos_api::ComputerAccess::Control,
        }),
    };
    state.runtimes().ensure_builtin(&builtins, "node").await.unwrap();
    let checked = state.runtimes().check("host", &["native-regression-tool".into()], false).await.unwrap();
    assert!(checked.usable());
    assert_eq!(checked.runtime_id, "private");
    assert!(checked.caps.desktop_error.is_some());
    assert_eq!(checked.caps.tools_check.unwrap().resolved["native-regression-tool"].path, tool.display().to_string());
    let ran = libopendan::runtime::runtime_exec(&state, "host", "native-regression-tool", None, 10_000, None).await.unwrap();
    assert_eq!(ran.runtime_id, "private");
    assert_eq!((ran.exit_code, ran.stdout.as_str()), (0, "native-managed-tool"));
    match old_root { Some(value) => std::env::set_var("BUCKYOS_ROOT", value), None => std::env::remove_var("BUCKYOS_ROOT") }
}

/// A desktop found unavailable is probed again once its check is older than
/// the recheck interval: what a probe saw while the executor still waited for
/// its first authorization is not what later sessions are told.
#[tokio::test]
async fn a_desktop_found_unavailable_is_probed_again() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let mut builtins = builtin(temp.path());
    builtins.truncate(1);
    builtins[0].default = true;
    builtins[0].connector = RuntimeConnector::Native {
        aliases: vec!["host".into()],
        computer: Some(agent_tool::runtime::computer::NativeComputerConfig {
            endpoint: temp.path().join("missing.sock").display().to_string(),
            node_did: "did:web:test".into(),
            profile_id: "agent-test".into(),
            access: buckyos_api::ComputerAccess::Control,
        }),
    };
    state.runtimes().ensure_builtin(&builtins, "node").await.unwrap();
    let live = state.runtimes().check("private", &[], false).await.unwrap().caps.desktop_error.expect("no executor answers");

    let file = state.layout().runtimes_dir().join("private.json");
    let stale = "local computer IPC: the executor has no valid authorization snapshot";
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    stored["caps"]["desktop_error"] = json!(stale);
    std::fs::write(&file, serde_json::to_vec(&stored).unwrap()).unwrap();
    let kept = libopendan::runtime::lookup_rechecked(&state, "private").await.unwrap().unwrap();
    assert_eq!(kept.caps.desktop_error.as_deref(), Some(stale), "checked moments ago");

    stored["checked_at_ms"] = json!(1);
    std::fs::write(&file, serde_json::to_vec(&stored).unwrap()).unwrap();
    let again = libopendan::runtime::lookup_rechecked(&state, "private").await.unwrap().unwrap();
    assert_eq!(again.caps.desktop_error, Some(live));
    assert!(again.checked_at_ms.unwrap() > 1);
}
