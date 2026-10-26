//! Session runtime and workspace decoupled (primary-runtime TODO §3–§6):
//! the session-runtime is chosen without the workspace, the session works
//! in its own directory, cmd_skills follow the session-runtime's declared
//! baseline, Session processes reach AgentService with a session token, and
//! files move between the session's runtimes by a verified copy.

mod common;

use std::path::{Path, PathBuf};

use buckyos_api::RuntimeBaseline;
use common::*;
use libopendan::api::{create_sub_session, SubSessionSpec, SubWorkspace};
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::session_context::{is_session_method, HelperOutput, SessionService, TokenKey};
use libopendan::state::AgentStateClient;
use libopendan::transfer::{copy, request_cancel, status, CopyRequest, Endpoint, TransferState};
use libopendan::OpenDanError;
use serde_json::json;

fn lab(baseline: RuntimeBaseline, default: bool) -> RuntimeRegister {
    RuntimeRegister {
        runtime_id: "lab".into(),
        kind: RuntimeKind::RemoteNode,
        name: "Lab node".into(),
        description: String::new(),
        exec: Some(node_exec("did:dev:lab", AGENT)),
        roots: vec![],
        capabilities: json!({}),
        default,
        computer: Default::default(),
        baseline,
    }
}

fn skill(env: &Env, name: &str, front: &str) {
    let dir = env.agent_root.join("cmd_skills");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.md")),
        format!("---\nname: {name}\ndescription: {name} things\n{front}---\nRun `{name}`.\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn the_workspace_never_chooses_the_session_runtime() {
    let env = Env::new();
    let agent = env.agent();
    agent.runtimes().register(&lab(RuntimeBaseline::Standard, true), APP).await.unwrap();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let workspace = env.workspace(&directory).await;

    // The agent's default runtime, with a workspace on the container.
    let mut spec = work_spec("default runtime, container workspace");
    spec.workspace = Some(workspace.clone());
    let sd = env.create_work(spec).await;
    let cfg = sd.config().unwrap();
    let frozen = cfg.runtime.frozen.clone().unwrap();
    assert_eq!((frozen.runtime_id.as_str(), frozen.selected_by), ("lab", RuntimeSelection::Default));
    assert_eq!(frozen.baseline, RuntimeBaseline::Standard);
    assert_eq!(cfg.workspace_binding.as_ref().unwrap().location.runtime_id, "local");
    let view = libopendan::runtime::workspace_view(&cfg).unwrap();
    assert_eq!(view["runtime_id"], "container");
    assert_eq!(view["same_runtime"], false);

    // An explicit runtime with the same workspace: no conflict.
    let mut spec = work_spec("container runtime");
    spec.workspace = Some(workspace.clone());
    spec.runtime.requirement.runtime_id = Some("container".into());
    let parent = env.create_work(spec).await;
    let pcfg = parent.config().unwrap();
    assert_eq!(pcfg.runtime.runtime_id(), "container");
    assert_eq!(pcfg.runtime.frozen.as_ref().unwrap().baseline, RuntimeBaseline::Extended);
    assert_eq!(libopendan::runtime::workspace_view(&pcfg).unwrap()["same_runtime"], true);

    // A sub session inherits the workspace and names another runtime; the
    // parent keeps both of its own.
    let ch = env.channels();
    let child = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "on the lab node".into(),
            runtime_id: Some("lab".into()),
            workspace: SubWorkspace::Inherit,
            key: Some("lab-child".into()),
            ..Default::default()
        },
        ch.as_ref(),
    )
    .await
    .unwrap();
    let ccfg = child.config().unwrap();
    assert_eq!(ccfg.runtime.runtime_id(), "lab");
    assert_eq!(ccfg.runtime.frozen.as_ref().unwrap().selected_by, RuntimeSelection::Explicit);
    assert_eq!(ccfg.workspace, pcfg.workspace);
    let inherited = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec { objective: "inherits".into(), key: Some("inherit-child".into()), ..Default::default() },
        ch.as_ref(),
    )
    .await
    .unwrap();
    let icfg = inherited.config().unwrap();
    assert_eq!(icfg.runtime.runtime_id(), "container");
    assert_eq!(icfg.runtime.frozen.as_ref().unwrap().selected_by, RuntimeSelection::Inherited);
}

#[tokio::test]
async fn a_session_works_in_its_own_directory_and_is_told_where_the_workspace_is() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = work_spec("look around");
    spec.workspace = Some(env.workspace(&directory).await);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call("pwd", "shell", json!({"command":"pwd > where.txt"})),
        _ => text("done"),
    });
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await.is_finished());
    let here = std::fs::read_to_string(sd.path().join("where.txt")).unwrap();
    assert_eq!(Path::new(here.trim()), sd.path().canonicalize().unwrap());
    let first = llm.requests.lock().unwrap()[0].clone();
    let text = serde_json::to_string(&first.messages).unwrap();
    assert!(text.contains("\\\"workspace\\\"") && text.contains("same_runtime"), "{text}");
    assert!(text.contains(&directory.canonicalize().unwrap().display().to_string()));
}

#[tokio::test]
async fn cmd_skills_follow_the_session_runtimes_declared_baseline() {
    let env = Env::new();
    let agent = env.agent();
    agent.runtimes().register(&lab(RuntimeBaseline::Standard, false), APP).await.unwrap();
    skill(&env, "plain", "");
    skill(&env, "media", "baseline: extended\n");
    skill(&env, "mac_only", "platforms: macos\n");
    let mut spec = work_spec("standard runtime");
    spec.runtime.requirement.runtime_id = Some("lab".into());
    spec.prompt.cmd_skills = vec!["plain".into(), "media".into()];
    spec.freeze = true;
    let sd = env.create_work(spec).await;
    let frozen = sd.config().unwrap().prompt.frozen.unwrap();
    assert_eq!(frozen.cmd_skills.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["plain"]);
    assert_eq!(frozen.unavailable_cmd_skills.len(), 1);
    assert_eq!(frozen.unavailable_cmd_skills[0].name, "media");
    assert!(frozen.unavailable_cmd_skills[0].reason.contains("extended"));

    // The container is extended and runs Linux: `media` applies, `mac_only`
    // does not.
    let mut spec = work_spec("container");
    spec.prompt.cmd_skills = vec!["media".into(), "mac_only".into()];
    spec.freeze = true;
    let parent = env.create_work(spec).await;
    let frozen = parent.config().unwrap().prompt.frozen.unwrap();
    assert_eq!(frozen.cmd_skills.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["media"]);
    assert!(frozen.unavailable_cmd_skills[0].reason.contains("macos"));

    // Work that names its manuals is refused on a runtime that cannot use
    // them; inherited manuals are resolved again for the child's runtime.
    let ch = env.channels();
    let err = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "needs media on lab".into(),
            runtime_id: Some("lab".into()),
            cmd_skills: Some(vec!["media".into()]),
            key: Some("strict".into()),
            ..Default::default()
        },
        ch.as_ref(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err:?}");
    assert!(err.to_string().contains("media") && err.to_string().contains("lab"), "{err}");
    let child = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "inherits media on lab".into(),
            runtime_id: Some("lab".into()),
            key: Some("lenient".into()),
            ..Default::default()
        },
        ch.as_ref(),
    )
    .await
    .unwrap();
    let frozen = child.config().unwrap().prompt.frozen.unwrap();
    assert!(frozen.cmd_skills.is_empty());
    assert_eq!(frozen.unavailable_cmd_skills[0].name, "media");

    // `skill list` shows what each needs; installing a bad value is refused.
    let listed = agent.behaviors().skills(Some(SkillKind::Cmd)).await.unwrap();
    let media = listed.iter().find(|s| s.name == "media").unwrap();
    assert_eq!(media.baseline, RuntimeBaseline::Extended);
    assert_eq!(listed.iter().find(|s| s.name == "mac_only").unwrap().platforms, ["macos"]);
    let pkg = libopendan::skills::SkillPackage {
        package: "bad".into(),
        files: [(
            "cmd_skills/bad.md".to_string(),
            libopendan::skills::SkillFile {
                content: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    "---\nbaseline: huge\n---\nx",
                ),
                executable: false,
            },
        )]
        .into(),
        ..Default::default()
    };
    let err = libopendan::skills::install(&env.agent_root, &pkg, false, APP).unwrap_err();
    assert!(err.to_string().contains("huge"), "{err}");
}

async fn helper_env() -> (Env, libopendan::SessionDir, SessionService) {
    let env = Env::new();
    let mut spec = work_spec("helpers from elsewhere");
    spec.input_channel = Some(libopendan::InputChannel::None);
    let sd = env.create_work(spec).await;
    let svc = SessionService::new(&env.agent_root, Some(PathBuf::from(env!("CARGO_BIN_EXE_xagent")))).unwrap();
    (env, sd, svc)
}

#[tokio::test]
async fn a_session_token_reaches_only_its_own_session_methods() {
    let (env, sd, svc) = helper_env().await;
    let agent = env.agent();
    let key = TokenKey::load_or_create(&env.agent_root).unwrap();
    let now = libopendan::now_ms();
    let token = key.sign(agent.agent_id(), sd.sid(), now + 60_000);
    let claims = svc.authenticate(agent.agent_id(), &token).unwrap();
    assert_eq!(claims.session_id, sd.sid());
    assert!(svc.authenticate("another-agent", &token).is_err());
    assert!(svc.authenticate(agent.agent_id(), &key.sign(agent.agent_id(), sd.sid(), now - 1)).is_err());
    assert!(is_session_method("session.helper") && !is_session_method("sessions.post_input"));

    // A forwarded helper runs as the session's own command on this side.
    let out: HelperOutput = serde_json::from_value(
        svc.call(
            agent.as_ref(),
            &claims,
            "session.helper",
            &json!({ "sid": sd.sid(), "request": { "argv": ["sessions"] } }),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(out.exit_code, 0, "{out:?}");
    assert!(out.stdout.contains(sd.sid()), "{out:?}");
    let out: HelperOutput = serde_json::from_value(
        svc.call(
            agent.as_ref(),
            &claims,
            "session.helper",
            &json!({ "request": { "argv": ["todo", "add", "first", "--content", "-"], "stdin": "from stdin" } }),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(out.exit_code, 0, "{out:?}");
    let list = libopendan::todos::store_of(&sd).load().unwrap();
    assert!(list.render_view().contains("first"));

    let other = svc
        .call(agent.as_ref(), &claims, "session.helper", &json!({ "sid": "another", "request": { "argv": ["sessions"] } }))
        .await
        .unwrap_err();
    assert!(other.to_string().contains("another") || other.to_string().contains(sd.sid()), "{other}");
    let task = svc
        .call(agent.as_ref(), &claims, "session.task", &json!({ "request": { "op": "state", "task_id": "t1" } }))
        .await
        .unwrap();
    assert_eq!(task["state"], "unknown");
    let renewed = svc
        .call(agent.as_ref(), &claims, "session.token_refresh", &json!({}))
        .await
        .unwrap();
    let fresh = renewed["token"].as_str().unwrap();
    assert_eq!(svc.authenticate(agent.agent_id(), fresh).unwrap().session_id, sd.sid());
}

fn endpoint(s: &str) -> Endpoint {
    s.parse().unwrap()
}

#[tokio::test]
async fn files_are_copied_between_the_session_and_its_workspace() {
    let env = Env::new();
    let directory = env.root.join("中文 project");
    std::fs::create_dir(&directory).unwrap();
    let workspace = env.workspace(&directory).await;
    let mut spec = work_spec("copy material");
    spec.workspace = Some(workspace.clone());
    let sd = env.create_work(spec).await;
    let agent = env.agent();
    let payload: Vec<u8> = (0..(9 * 1024 * 1024 + 77)).map(|i| (i % 253) as u8).collect();
    std::fs::write(directory.join("素材 a.bin"), &payload).unwrap();

    let req = |from: &str, to: &str, replace: bool, call: Option<&str>| CopyRequest {
        from: endpoint(from),
        to: endpoint(to),
        replace,
        call_id: call.map(str::to_string),
    };
    let got = copy(agent.as_ref(), &sd, &req("workspace:素材 a.bin", "session:in/a.bin", false, Some("c1")))
        .await
        .unwrap();
    assert_eq!(got.state, TransferState::Committed, "{got:?}");
    assert_eq!((got.size, got.bytes_done), (payload.len() as u64, payload.len() as u64));
    assert_eq!(std::fs::read(sd.path().join("in/a.bin")).unwrap(), payload);
    assert!(got.hash.as_deref().unwrap().starts_with("sha256:"));
    assert_eq!(got.to.runtime_id, "container");
    assert_eq!(status(&sd, &got.transfer_id).unwrap(), got);
    // The same call again: the committed copy is checked, not redone.
    let again = copy(agent.as_ref(), &sd, &req("workspace:素材 a.bin", "session:in/a.bin", false, Some("c1")))
        .await
        .unwrap();
    assert_eq!(again, got);

    // Back into the workspace: an existing target only with --replace.
    std::fs::write(sd.path().join("in/a.bin"), b"processed").unwrap();
    let refused = copy(agent.as_ref(), &sd, &req("session:in/a.bin", "workspace:素材 a.bin", false, None))
        .await
        .unwrap();
    assert_eq!(refused.state, TransferState::Failed);
    assert_eq!(refused.step.as_deref(), Some("commit"));
    assert!(refused.error.as_deref().unwrap().contains("--replace"));
    assert_eq!(std::fs::read(directory.join("素材 a.bin")).unwrap(), payload, "untouched");
    assert!(std::fs::read_dir(&directory).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));
    let replaced = copy(agent.as_ref(), &sd, &req("session:in/a.bin", "workspace:out/", true, None))
        .await
        .unwrap();
    assert_eq!(replaced.state, TransferState::Committed);
    assert_eq!(std::fs::read(directory.join("out/a.bin")).unwrap(), b"processed");

    // What a copy does not do.
    for (from, to, why) in [
        ("workspace:missing.bin", "session:x", "missing.bin"),
        ("workspace:out", "session:x", "not a regular file"),
        ("session:../escape", "session:x", ".."),
        ("session:in/a.bin", "session:in/a.bin", "same file"),
        ("nowhere:/x", "session:x", "nowhere"),
        ("container:relative", "session:x", "absolute"),
    ] {
        let err = copy(agent.as_ref(), &sd, &req(from, to, false, None)).await.unwrap_err();
        assert!(err.to_string().contains(why), "{from} → {to}: {err}");
    }
    let mut readonly = work_spec("read only");
    readonly.workspace = Some(WorkspaceRef { access: WorkspaceAccess::ReadOnly, ..workspace });
    let ro = env.create_work(readonly).await;
    let err = copy(agent.as_ref(), &ro, &req("session:x", "workspace:y", false, None)).await.unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
    assert!(request_cancel(&sd, &got.transfer_id).unwrap().state == TransferState::Committed);
}

#[tokio::test]
async fn agent_session_copy_runs_in_the_session() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("in.txt"), "material").unwrap();
    let mut spec = work_spec("copy by command");
    spec.workspace = Some(env.workspace(&directory).await);
    let sd = env.create_work(spec).await;
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["copy", "--from", "workspace:in.txt", "--to", "session:got.txt"])
        .current_dir(sd.path())
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .env("OPENDAN_SESSION_ID", sd.sid())
        .env("LIBOPENDAN_QUEUE_DIR", &env.queue_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let record: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(record["state"], "committed");
    assert_eq!(std::fs::read_to_string(sd.path().join("got.txt")).unwrap(), "material");
}
