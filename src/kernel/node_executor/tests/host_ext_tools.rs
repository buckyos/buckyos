//! A BuckyOS host's tool layers as an Agent's checks and commands see them, through a real executor running as the
//! current user: `tools/bin` first, `bin/agent-tools` next, the profile's PATH last (ext-tools TODO §3.6, P1-5).

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use agent_tool::runtime::node::{NodeExecutorConnector, NodeIo};
use async_trait::async_trait;
use buckyos_api::*;
use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use node_executor::{ExecutorAdmission, ExecutorHost, ExecutorIdentityMode, ExecutorOptions, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher};

const APP: &str = "jarvis.test.buckyos.io";
const NODE_DID: &str = "did:web:ood1.test.buckyos.io";

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

fn script(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checks_and_commands_see_the_same_layers_of_a_host() {
    let dir = tempfile::Builder::new().prefix("nx-ext").tempdir_in("/tmp").unwrap();
    let host = dir.path();
    let root = host.join("buckyos");
    let tools_bin = root.join("tools/bin");
    let agent_tools = root.join("bin/agent-tools");
    let system = host.join("system-bin");
    let work = host.join("work");
    std::fs::create_dir_all(&work).unwrap();

    for name in libopendan::baseline::required_commands(RuntimeBaseline::Standard, false) {
        script(&agent_tools, name, "exit 0");
    }
    // Extension tools come first: this ffmpeg shadows the machine's.
    script(&tools_bin, "ffmpeg", "echo 'ffmpeg version 7.1'");
    script(&system, "ffmpeg", "echo 'ffmpeg version 1.0'");
    script(&tools_bin, "pandoc", "echo 'not bundled' >&2; exit 127");
    for tool in libopendan::baseline::EXTENDED_TOOLS {
        for command in tool.commands {
            if !["ffmpeg", "pandoc"].contains(&command.name) {
                script(&system, command.name, "echo 1.0");
            }
        }
    }

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
            path_prefix: vec![tools_bin.clone(), agent_tools.clone()],
            buckyos_root: Some(root.clone()),
        },
    )
    .unwrap();
    let agent_did = AppId::parse(APP).unwrap().app_did().to_string();
    let snapshot = ExecutorConfig {
        enabled: true,
        revision: "sha256:r1".into(),
        generation: 1,
        auth_valid_until_ms: node_executor::now_ms() + 30_000,
        allowed_devices: vec!["did:dev:peer".into()],
        principal_bindings: BTreeMap::from([(
            agent_did.clone(),
            ExecutorPrincipalBinding {
                app_instance_id: format!("{APP}@devtest").parse().unwrap(),
                agent_generation: 1,
                binding_generation: 1,
                enabled: true,
                app_installed: true,
                permission_mode: AgentPermissionMode::ReadWrite,
            },
        )]),
        profiles: BTreeMap::from([(
            "dev".into(),
            ExecutorProfile {
                baseline: None,
                run_as: None,
                workdir_roots: vec![work.to_string_lossy().to_string()],
                env: BTreeMap::from([("PATH".into(), format!("{}:/usr/bin:/bin", system.display()))]),
                principals: vec![agent_did.clone()],
                ops: ExecutorOp::ALL.to_vec(),
            },
        )]),
        limits: ExecutorLimits::default(),
    };
    exec.apply_config(Some(snapshot)).await;
    libopendan::runtime::set_node_connector(Arc::new(InProcess { exec: exec.clone() }));

    let agent_root = host.join("agent root");
    let agent = libopendan::FsAgentStateClient::open(&agent_root, &agent_did, None, None).unwrap();
    let register = |runtime_id: &str, baseline| RuntimeRegister {
        runtime_id: runtime_id.into(),
        connector: RuntimeConnector::NodeExecutor { node_did: Some(NODE_DID.into()), executor_id: None, profile_id: "dev".into() },
        name: "Host".into(),
        desc: String::new(),
        enabled: true,
        default: false,
        baseline,
        grant: RuntimeGrant { roots: vec![work.clone()], ..Default::default() },
    };
    agent.runtimes().register(&register("box", RuntimeBaseline::Extended), "owner").await.unwrap();
    agent.runtimes().register(&register("plain", RuntimeBaseline::Standard), "owner").await.unwrap();

    // A background check only looks at paths: the stub passes for now.
    let checked = agent.runtimes().check("box", &[], false).await.unwrap();
    assert_eq!(checked.availability, RuntimeAvailability::Available, "{:?}", checked.last_error);
    assert_eq!(checked.caps.buckyos_root.as_deref(), Some(root.to_str().unwrap()), "describe reports the root");
    let check = checked.caps.baseline_check.clone().unwrap();
    assert!(check.error.is_none(), "{check:?}");
    assert_eq!((check.level, check.ok), (RuntimeBaseline::Extended, true), "{check:?}");
    let ffmpeg = check.resolved["ffmpeg"].clone();
    assert_eq!((ffmpeg.path.as_str(), ffmpeg.provided_by), (tools_bin.join("ffmpeg").to_str().unwrap(), ProvidedBy::Exttools));
    assert_eq!(ffmpeg.version, None, "a background check records paths only");
    assert_eq!(check.resolved["xagent"].provided_by, ProvidedBy::BuckyosRoot);
    assert_eq!(check.resolved["python3"].provided_by, ProvidedBy::System);
    assert_eq!(check.ext.as_ref().unwrap().summary(), "system");
    assert!(libopendan::runtime::runtime_line(&checked).contains("; ext: system"));

    // The session's shell finds the very same ffmpeg (P1-5).
    let op = libopendan::runtime::runtime_exec(&agent, "box", "command -v ffmpeg; ffmpeg", Some(work.to_str().unwrap()), 10_000, None).await.unwrap();
    assert_eq!(op.exit_code, 0, "{op:?}");
    assert_eq!(op.stdout.trim(), format!("{}\nffmpeg version 7.1", ffmpeg.path), "{op:?}");

    // A full check runs the tools: versions are recorded, the stub is found out.
    let full = agent.runtimes().check("box", &["pandoc".into(), "ffmpeg".into(), "no-such-tool-xyz".into()], true).await.unwrap();
    let check = full.caps.baseline_check.clone().unwrap();
    assert!(!check.ok && check.missing == vec!["pandoc".to_string()], "{check:?}");
    assert_eq!(check.resolved["ffmpeg"].version.as_deref(), Some("ffmpeg version 7.1"));
    assert!(check.resolved["pandoc"].stub, "{check:?}");
    assert!(check.ext.as_ref().unwrap().summary().contains("partial(missing pandoc)"));
    let tools = full.caps.tools_check.clone().unwrap();
    assert_eq!(tools.present, vec!["ffmpeg".to_string()]);
    assert_eq!(tools.missing, vec!["pandoc".to_string(), "no-such-tool-xyz".to_string()]);
    assert_eq!(tools.resolved["ffmpeg"].version.as_deref(), Some("ffmpeg version 7.1"));
    assert_eq!(full.caps.baseline, RuntimeBaseline::Extended, "a check never changes the declaration");

    // A later background check keeps what the full one learned of the same file.
    let again = agent.runtimes().check("box", &[], false).await.unwrap();
    let check = again.caps.baseline_check.unwrap();
    assert_eq!(check.resolved["ffmpeg"].version.as_deref(), Some("ffmpeg version 7.1"));
    assert!(check.resolved["pandoc"].stub && !check.ok, "{check:?}");

    // The extended layer does not bind a runtime declared standard, and is still shown.
    let plain = agent.runtimes().check("plain", &[], true).await.unwrap();
    let check = plain.caps.baseline_check.unwrap();
    assert!(check.ok && check.missing.is_empty(), "{check:?}");
    assert!(check.ext.unwrap().summary().starts_with("partial"));

    // The user's own `xagent` first on PATH is not the standard one: its dependencies are not the pinned ones.
    std::fs::remove_file(agent_tools.join("xagent")).unwrap();
    script(&system, "xagent", "exit 0");
    let shadowed = agent.runtimes().check("plain", &[], false).await.unwrap();
    let check = shadowed.caps.baseline_check.unwrap();
    assert!(!check.ok, "{check:?}");
    assert_eq!(check.unmanaged, vec!["xagent".to_string()]);
    assert_eq!(check.resolved["xagent"].provided_by, ProvidedBy::System);
    assert!(check.problem().unwrap().contains("not from $BUCKYOS_ROOT: xagent"));

    // A missing one is plain missing.
    std::fs::remove_file(system.join("xagent")).unwrap();
    let check = agent.runtimes().check("plain", &[], false).await.unwrap().caps.baseline_check.unwrap();
    assert_eq!(check.missing, vec!["xagent".to_string()]);
}
