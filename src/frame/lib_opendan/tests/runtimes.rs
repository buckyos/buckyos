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
    builtin_runtimes(vec![path.join("workspaces")], "")
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
    assert!(host.exec.is_none() && host.last_error.as_deref().unwrap().contains("no executor"));

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
