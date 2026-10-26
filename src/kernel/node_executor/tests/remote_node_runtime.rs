//! agent_tool's `remote_node` runtime against a real executor (in-process
//! streams, real workers as uid 65534). Needs root.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_tool::exec_tracking::InflightAction;
use agent_tool::llm_bash::{BashRunRequest, BashTarget};
use agent_tool::runtime::node::{NodeExecutorConnector, NodeIo};
use agent_tool::runtime::{RemoteNodeConfig, RuntimeConfig, RuntimeOpenCtx, RuntimeRegistry, Sandbox};
use agent_tool::xllm::{LoopModel, ToolsConfig, XllmDeps};
use agent_tool::SessionRuntimeContext;
use async_trait::async_trait;
use buckyos_api::*;
use llm_context::deps::{ToolCallCtx, ToolManager};
use node_executor::{ExecutorHost, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher};
use serde_json::{json, Value};

const NOBODY: u32 = 65534;
const APP: &str = "jarvis.test.buckyos.io";
const NODE_DID: &str = "did:web:ood1.test.buckyos.io";

extern "C" {
    fn geteuid() -> u32;
}

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

/// Serves each connection with whichever executor is current.
struct InProcess {
    exec: Mutex<NodeExecutor>,
}

#[async_trait]
impl NodeExecutorConnector for InProcess {
    async fn connect(&self, node_did: &str) -> std::io::Result<Box<dyn NodeIo>> {
        assert_eq!(node_did, NODE_DID);
        let (a, b) = tokio::io::duplex(1 << 20);
        let exec = self.exec.lock().unwrap().clone();
        tokio::spawn(async move { exec.serve(b).await });
        Ok(Box::new(a))
    }
    async fn session_token(&self) -> Result<String, String> {
        Ok(format!("{APP}@devtest"))
    }
}

fn snapshot(work: &Path) -> ExecutorConfig {
    let agent = AppId::parse(APP).unwrap().app_did().to_string();
    ExecutorConfig {
        enabled: true,
        revision: "sha256:r1".into(),
        generation: 1,
        auth_valid_until_ms: node_executor::now_ms() + 30_000,
        allowed_devices: vec!["did:dev:peer".into()],
        principal_bindings: BTreeMap::from([(
            agent.clone(),
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
                run_as: Some(ExecutorRunAs { uid: NOBODY, gid: NOBODY }),
                workdir_roots: vec![work.to_string_lossy().to_string()],
                env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                principals: vec![agent],
                ops: ExecutorOp::ALL.to_vec(),
            },
        )]),
        limits: ExecutorLimits::default(),
    }
}

async fn executor(root: &Path, ledger: &str, work: &Path) -> NodeExecutor {
    let worker = root.join("node_executor_worker");
    if !worker.exists() {
        std::fs::copy(env!("CARGO_BIN_EXE_node_executor_worker"), &worker).unwrap();
        std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let exec = NodeExecutor::open(ExecutorHost {
        node_id: "ood1".into(),
        node_did: NODE_DID.into(),
        store_dir: root.join(ledger),
        worker: WorkerLauncher { program: worker, args: vec![] },
        shell: node_executor::default_shell(),
        verifier: Arc::new(Verifier),
        rbac: Arc::new(AllowAll),
    })
    .unwrap();
    exec.apply_config(Some(snapshot(work))).await;
    exec
}

fn call(id: &str, name: &str, args: Value) -> AiToolCall {
    AiToolCall {
        call_id: id.into(),
        name: name.into(),
        args: args.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
    }
}

fn ctx() -> SessionRuntimeContext {
    SessionRuntimeContext {
        trace_id: "run1".into(),
        agent_name: "test".into(),
        behavior: "test".into(),
        tool_call_index: 0,
        wakeup_id: String::new(),
        session_id: "run1".into(),
        read_token_limit: 100_000,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_node_tools_run_on_the_node_and_recover_by_query() {
    if unsafe { geteuid() } != 0 {
        eprintln!("skipping: needs root");
        return;
    }
    let dir = tempfile::Builder::new().prefix("nx-rt").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let root = dir.path().to_path_buf();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    std::os::unix::fs::chown(&work, Some(NOBODY), Some(NOBODY)).unwrap();
    let local = root.join("local");
    std::fs::create_dir(&local).unwrap();
    let run_dir: PathBuf = local.join("runs").join("run1");

    let connector = Arc::new(InProcess {
        exec: Mutex::new(executor(&root, "ledger-a", &work).await),
    });
    let deps = XllmDeps::default().with_node_executor(connector.clone());
    let config = RuntimeConfig {
        kind: Some("remote_node".into()),
        workdir: Some(work.to_string_lossy().to_string()),
        remote_node: Some(RemoteNodeConfig {
            node_did: Some(NODE_DID.into()),
            profile_id: Some("dev".into()),
        }),
        ..Default::default()
    };
    assert!(RuntimeRegistry::from_config(&config).is_err(), "no connector, no runtime");
    let rt = RuntimeRegistry::from_config_with(&config, &deps).unwrap();
    let open = RuntimeOpenCtx::new(&local, "run1", LoopModel::FunctionCall);
    let (_, mut manager) = rt
        .open(&open, &ToolsConfig { enabled: Some(true), ..Default::default() }, &deps)
        .await
        .unwrap();
    manager.bind_run_dir("run1", Some(run_dir.clone()));
    let descriptor = rt.descriptor().clone();
    assert_eq!(descriptor.kind, "remote_node");
    assert_eq!(descriptor.target["uid"], json!(NOBODY));
    assert_eq!(descriptor.target["node_did"], json!(NODE_DID));

    let obs = manager
        .call_tool(call("c-shell", "shell", json!({"command": "id -u; echo hello > out.txt; cat out.txt"})), ToolCallCtx::noop())
        .await
        .unwrap();
    let text = format!("{obs:?}");
    assert!(text.contains("65534") && text.contains("hello"), "{text}");
    assert!(run_dir.join("runtime_operations").join("c-shell.json").exists());

    let obs = manager
        .call_tool(call("c-write", "write_file", json!({"path": "a.txt", "content": "one\ntwo\n"})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(!matches!(obs, llm_context::observation::Observation::Error { .. }), "{obs:?}");
    use std::os::unix::fs::MetadataExt;
    assert_eq!(std::fs::metadata(work.join("a.txt")).unwrap().uid(), NOBODY);

    let obs = manager
        .call_tool(call("c-edit", "edit_file", json!({"path": "a.txt", "old_string": "two", "new_string": "TWO"})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("TWO"), "{obs:?}");
    let obs = manager
        .call_tool(call("c-edit2", "edit_file", json!({"path": "a.txt", "old_string": "missing", "new_string": "x"})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("not found"), "{obs:?}");

    let obs = manager
        .call_tool(call("c-read", "read_file", json!({"path": "a.txt"})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("one\\nTWO"), "{obs:?}");
    assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "one\nTWO\n");

    // Template __EXEC__ has no call id: refused before anything runs.
    let err = manager
        .exec(
            BashRunRequest {
                command: "touch should-not-exist".into(),
                cwd: work.clone(),
                timeout_ms: 5_000,
                max_output_bytes: 4096,
                env: Vec::new(),
                target: BashTarget::Local,
                call_id: None,
            },
            &ctx(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Capability"), "{err}");
    assert!(!work.join("should-not-exist").exists());

    // A crashed runner only queries what its locator names.
    let run = agent_tool::llm_bash::RunBinding {
        run_id: "run1".into(),
        run_dir: Some(run_dir.clone()),
    };
    let inflight = |id: &str| InflightAction {
        call_id: id.into(),
        tool: "shell".into(),
        args: json!({"command": "x"}),
        effect: "unknown".into(),
        idempotency_key: None,
        step_index: None,
        started_at_ms: node_executor::now_ms(),
    };
    let text = rt.describe_interrupted(&run, &inflight("c-shell")).await;
    assert!(text.contains("Succeeded") && text.contains("hello"), "{text}");
    let text = rt.describe_interrupted(&run, &inflight("never-sent")).await;
    assert!(text.contains("did not run"), "{text}");

    // A command outliving wait_ms becomes a task; a fresh runner (new
    // process) learns its state from the locator and the node.
    let obs = manager
        .call_tool(call("c-long", "shell", json!({"command": "sleep 2; echo late", "wait_ms": 300})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("local:shell:c-long"), "{obs:?}");
    let rt2 = RuntimeRegistry::from_config_with(&config, &deps).unwrap();
    let open2 = RuntimeOpenCtx::new(&local, "run1", LoopModel::FunctionCall);
    let (_, mut manager2) = rt2
        .open(&open2, &ToolsConfig { enabled: Some(true), ..Default::default() }, &deps)
        .await
        .unwrap();
    manager2.bind_run_dir("run1", Some(run_dir.clone()));
    let resolver = manager2.resolver();
    let mut finished = None;
    for _ in 0..40 {
        match resolver.state("local:shell:c-long").await {
            llm_context::tasks::TaskState::Finished(r) => {
                finished = Some(r);
                break;
            }
            llm_context::tasks::TaskState::Running { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let finished = finished.expect("task finished");
    assert!(finished.success && finished.output.contains("late"), "{finished:?}");

    // The ledger is replaced: old calls are not resumed under the new store.
    let old = connector.exec.lock().unwrap().clone();
    old.apply_config(None).await;
    *connector.exec.lock().unwrap() = executor(&root, "ledger-b", &work).await;
    let text = rt.describe_interrupted(&run, &inflight("c-shell")).await;
    assert!(text.contains("reset"), "{text}");
    let obs = manager
        .call_tool(call("c-after", "shell", json!({"command": "echo after"})), ToolCallCtx::noop())
        .await
        .unwrap();
    assert!(format!("{obs:?}").contains("StoreMismatch") || format!("{obs:?}").contains("ledger changed"), "{obs:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_artifacts_download_in_ranges_without_truncation_or_workspace_escape() {
    if unsafe { geteuid() } != 0 {
        eprintln!("skipping: needs root");
        return;
    }
    let dir = tempfile::Builder::new().prefix("nx-artifacts").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let root = dir.path();
    let work = root.join("work");
    let session = work.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let source = session.join("video.mp4");
    let payload: Vec<u8> = (0..3 * 1024 * 1024 + 123).map(|i| (i % 251) as u8).collect();
    std::fs::write(&source, &payload).unwrap();
    std::fs::write(session.join("empty"), []).unwrap();
    std::fs::write(work.join("outside.mp4"), b"outside this session").unwrap();
    std::os::unix::fs::symlink(work.join("outside.mp4"), session.join("escape.mp4")).unwrap();
    let connector = Arc::new(InProcess { exec: Mutex::new(executor(root, "ledger", &work).await) });
    let target = agent_tool::runtime::node::RemoteNodeTarget::open(
        connector, NODE_DID, "dev", "host", root.join("ops"),
    ).await.unwrap();
    let output = root.join("download.mp4");
    let file = target.download_file(
        session.to_str().unwrap(), source.to_str().unwrap(), &output,
    ).await.unwrap();
    assert_eq!(file.size, payload.len() as u64);
    assert_eq!(file.hash, executor_content_hash(&payload));
    assert_eq!(std::fs::read(output).unwrap(), payload);
    let empty = root.join("empty");
    let file = target.download_file(
        session.to_str().unwrap(), session.join("empty").to_str().unwrap(), &empty,
    ).await.unwrap();
    assert_eq!(file.size, 0);
    assert_eq!(file.hash, executor_content_hash(&[]));
    assert_eq!(std::fs::metadata(empty).unwrap().len(), 0);
    for name in ["escape.mp4", "missing.mp4", "."] {
        let output = root.join("rejected");
        assert!(target.download_file(
            session.to_str().unwrap(), session.join(name).to_str().unwrap(), &output,
        ).await.is_err(), "{name}");
        assert!(!output.exists(), "{name}");
    }
}

/// What a copy between runtimes does with the primitives (TODO §4): stream
/// ranges into a temporary file guarded by its size, verify the whole hash,
/// commit by rename.
async fn copy_between(
    src: &dyn agent_tool::runtime::files::RuntimeFiles,
    from: &str,
    dst: &dyn agent_tool::runtime::files::RuntimeFiles,
    to: &str,
) -> String {
    use agent_tool::runtime::files::{WholeWriteMode, WriteChunk};
    let chunk = src.limits().max_chunk_bytes.min(dst.limits().max_chunk_bytes);
    let source = src.stat(from, true).await.unwrap();
    let part = format!("{to}.part");
    let mut offset = 0;
    loop {
        let range = src.read_range(from, offset, chunk).await.unwrap();
        assert_eq!(range.file_size, source.size);
        let mode = if offset == 0 { WholeWriteMode::CreateNew } else { WholeWriteMode::Append };
        let written = dst
            .write_chunk(&part, WriteChunk { mode, expected_size: (offset > 0).then_some(offset), expected_hash: None, data: &range.data })
            .await
            .unwrap();
        offset += range.data.len() as u64;
        assert_eq!(written.size, offset);
        if offset >= source.size {
            break;
        }
    }
    let landed = dst.stat(&part, true).await.unwrap();
    assert_eq!(landed.hash, source.hash);
    dst.rename(&part, to, false).await.unwrap();
    landed.hash.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_files_copy_both_ways_between_native_and_remote_node() {
    use agent_tool::runtime::files::{LocalFileBackend, PathType, RuntimeFiles, WholeWriteMode, WriteChunk};
    if unsafe { geteuid() } != 0 {
        eprintln!("skipping: needs root");
        return;
    }
    let dir = tempfile::Builder::new().prefix("nx-copy").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let root = dir.path();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    std::os::unix::fs::chown(&work, Some(NOBODY), Some(NOBODY)).unwrap();
    let local = root.join("local 本地");
    std::fs::create_dir(&local).unwrap();
    let connector = Arc::new(InProcess { exec: Mutex::new(executor(root, "ledger", &work).await) });
    let node = agent_tool::runtime::node::RemoteNodeTarget::open(connector, NODE_DID, "dev", "host", root.join("ops"))
        .await
        .unwrap();
    let native = LocalFileBackend;
    assert_eq!(node.limits().max_chunk_bytes, ExecutorLimits::default().max_file_bytes);

    let payload: Vec<u8> = (0..2 * 1024 * 1024 + 4321).map(|i| (i % 251) as u8).collect();
    let source = local.join("素材 a.bin");
    std::fs::write(&source, &payload).unwrap();
    let remote = work.join("in").join("素材 a.bin");
    let s = |p: &Path| p.to_string_lossy().to_string();
    let hash = copy_between(&native, &s(&source), &node, &s(&remote)).await;
    assert_eq!(hash, executor_content_hash(&payload));
    assert_eq!(std::fs::read(&remote).unwrap(), payload);
    assert_eq!(node.stat(&format!("{}.part", s(&remote)), false).await.unwrap().path_type, PathType::Missing);

    let back = local.join("back.bin");
    assert_eq!(copy_between(&node, &s(&remote), &native, &s(&back)).await, hash);
    assert_eq!(std::fs::read(&back).unwrap(), payload);
    let empty = local.join("empty");
    std::fs::write(&empty, []).unwrap();
    copy_between(&native, &s(&empty), &node, &s(&work.join("empty"))).await;
    assert_eq!(std::fs::metadata(work.join("empty")).unwrap().len(), 0);

    for (files, base) in [(&native as &dyn RuntimeFiles, &local), (&node, &work)] {
        let (a, b) = (base.join("x"), base.join("y"));
        let write = |path: String, mode, expected_size| async move {
            files.write_chunk(&path, WriteChunk { mode, expected_size, expected_hash: None, data: b"data" }).await
        };
        write(s(&a), WholeWriteMode::CreateNew, None).await.unwrap();
        write(s(&b), WholeWriteMode::Replace, None).await.unwrap();
        assert_eq!(write(s(&a), WholeWriteMode::CreateNew, None).await.unwrap_err().code, ExecutorErrorCode::Conflict);
        assert_eq!(write(s(&a), WholeWriteMode::Append, Some(3)).await.unwrap_err().code, ExecutorErrorCode::Conflict);
        assert_eq!(files.rename(&s(&a), &s(&b), false).await.unwrap_err().code, ExecutorErrorCode::Conflict);
        files.rename(&s(&a), &s(&b), true).await.unwrap();
        assert_eq!(files.rename(&s(&a), &s(&b), true).await.unwrap_err().code, ExecutorErrorCode::NotFound);
        assert!(files.remove(&s(&b)).await.unwrap());
        assert!(!files.remove(&s(&b)).await.unwrap());
        let too_big = files.limits().max_chunk_bytes + 1;
        assert_eq!(files.read_range(&s(&back), 0, too_big).await.unwrap_err().code, ExecutorErrorCode::InvalidArgument);
    }
    assert_eq!(node.remove(&s(&back)).await.unwrap_err().code, ExecutorErrorCode::Forbidden);
    assert!(back.exists());
}

/// A node as a session-runtime of an agent (primary-runtime TODO §4–§6):
/// its check learns the platform and finds the standard tools missing, a
/// session copies files to it and back through libopendan's transfer, and
/// the session's context lands in its working directory there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_session_runtime_is_checked_copied_to_and_given_its_context() {
    use libopendan::protocol::*;
    use libopendan::state::AgentStateClient;
    use libopendan::transfer::{copy, CopyRequest, TransferState};
    if unsafe { geteuid() } != 0 {
        eprintln!("skipping: needs root");
        return;
    }
    let dir = tempfile::Builder::new().prefix("nx-sr").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let root = dir.path();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    std::os::unix::fs::chown(&work, Some(NOBODY), Some(NOBODY)).unwrap();
    let connector = Arc::new(InProcess { exec: Mutex::new(executor(root, "ledger", &work).await) });
    libopendan::runtime::set_node_connector(connector);
    let agent_did = AppId::parse(APP).unwrap().app_did().to_string();
    let agent_root = root.join("agent root");
    let agent = libopendan::FsAgentStateClient::open(&agent_root, &agent_did, None, None).unwrap();
    let exec = json!({"kind": "remote_node", "remote_node": {"node_did": NODE_DID, "profile_id": "dev"}});
    agent
        .runtimes()
        .register(
            &RuntimeRegister {
                runtime_id: "lab".into(),
                kind: RuntimeKind::RemoteNode,
                name: "Lab".into(),
                description: String::new(),
                exec: Some(exec),
                roots: vec![],
                capabilities: json!({}),
                default: false,
                computer: Default::default(),
                baseline: RuntimeBaseline::Standard,
            },
            "owner",
        )
        .await
        .unwrap();
    let lab = agent.runtimes().check("lab").await.unwrap();
    assert_eq!(lab.availability, RuntimeAvailability::Available);
    assert_eq!(lab.roots, vec![work.clone()]);
    assert_eq!(lab.platform.os, std::env::consts::OS.replace("darwin", "macos"));
    let checked = lab.baseline_check.clone().unwrap();
    assert!(checked.error.is_none(), "{checked:?}");
    assert!(!checked.ok && checked.missing.iter().any(|m| m == "agent-session"), "{checked:?}");
    assert_eq!(lab.baseline, RuntimeBaseline::Standard, "the check never changes the declaration");

    let channels = libopendan::channel::KmsgChannels::dir(root.join("kmsg")).unwrap();
    let mut spec = libopendan::SessionTemplate::load("work", None).unwrap().spec("move material");
    spec.input_channel = Some(libopendan::InputChannel::None);
    let app_dir = root.join("sessions");
    std::fs::create_dir_all(&app_dir).unwrap();
    let sd = libopendan::create_session(&app_dir, spec, &agent, "app:test@devtest", &channels).await.unwrap();
    let payload: Vec<u8> = (0..3 * 1024 * 1024 + 17).map(|i| (i % 249) as u8).collect();
    std::fs::write(sd.path().join("素材 a.bin"), &payload).unwrap();
    let remote = work.join("in").join("素材 a.bin");
    let s = |p: &Path| p.to_string_lossy().to_string();
    let req = |from: String, to: String, replace: bool| CopyRequest {
        from: from.parse().unwrap(),
        to: to.parse().unwrap(),
        replace,
        call_id: None,
    };
    let there = copy(&agent, &sd, &req("session:素材 a.bin".into(), format!("lab:{}", s(&remote)), false)).await.unwrap();
    assert_eq!(there.state, TransferState::Committed, "{there:?}");
    assert_eq!(there.hash.as_deref(), Some(executor_content_hash(&payload).as_str()));
    assert_eq!(std::fs::read(&remote).unwrap(), payload);
    let back = copy(&agent, &sd, &req(format!("lab:{}", s(&remote)), "session:back/".into(), false)).await.unwrap();
    assert_eq!(back.state, TransferState::Committed, "{back:?}");
    assert_eq!(std::fs::read(sd.path().join("back").join("素材 a.bin")).unwrap(), payload);
    std::fs::write(sd.path().join("small.txt"), "v2").unwrap();
    let refused = copy(&agent, &sd, &req("session:small.txt".into(), format!("lab:{}", s(&remote)), false)).await.unwrap();
    assert_eq!((refused.state, refused.step.as_deref()), (TransferState::Failed, Some("commit")));
    assert_eq!(std::fs::read(&remote).unwrap(), payload, "the target is unchanged");
    assert!(!std::fs::read_dir(work.join("in")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".part")));
    let replaced = copy(&agent, &sd, &req("session:small.txt".into(), format!("lab:{}", s(&remote)), true)).await.unwrap();
    assert_eq!(replaced.state, TransferState::Committed);
    assert_eq!(std::fs::read_to_string(&remote).unwrap(), "v2");
    let outside = copy(&agent, &sd, &req("session:small.txt".into(), "lab:/etc/x".into(), false)).await.unwrap();
    assert_eq!(outside.state, TransferState::Failed);
    assert!(outside.error.as_deref().unwrap().contains("Forbidden"), "{outside:?}");

    // The context a helper there finds.
    let key = libopendan::session_context::TokenKey::load_or_create(&agent_root).unwrap();
    let workdir = s(&work.join(".opendan").join(&agent.agent_id()).join("sessions").join(sd.sid()));
    let cfg = sd.config().unwrap();
    let (ctx, token) = libopendan::session_context::new_context(&key, agent.agent_id(), &cfg, &workdir, "http://gw/kapi/x".into());
    let target = libopendan::runtime::open_node_target(&lab, &root.join("ops")).await.unwrap();
    libopendan::session_context::write_context(&target, &ctx, &token).await.unwrap();
    let (path, found) = libopendan::session_context::discover(None, Path::new(&workdir)).unwrap();
    assert_eq!(found, ctx);
    let on_disk = std::fs::read_to_string(libopendan::session_context::token_path(&path, &found)).unwrap();
    assert_eq!(key.verify(agent.agent_id(), &on_disk, libopendan::now_ms()).unwrap().session_id, sd.sid());
}
