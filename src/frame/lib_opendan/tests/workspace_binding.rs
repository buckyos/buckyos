mod common;

use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use common::*;
use libopendan::api::{create_session, create_sub_session, SubSessionSpec, SubWorkspace};
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;
use serde_json::{json, Value};
use std::time::Duration;

fn batch(calls: &[(&str, Value)]) -> AiResponse {
    AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        calls
            .iter()
            .map(|(id, args)| {
                AiContent::tool_use(
                    *id,
                    "shell",
                    args.as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )
            })
            .collect(),
    ))
}

#[tokio::test]
async fn an_unbound_session_keeps_its_own_directory() {
    let env = Env::new();
    let sd = env.create_work(work_spec("one-off task")).await;
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call(
            "write",
            "shell",
            json!({"command":"echo temporary > result.txt"}),
        ),
        _ => text("done"),
    });
    assert!(drive(&sd, &env.deps(llm), StopWhen::Finished)
        .await
        .is_finished());
    assert!(sd.config().unwrap().workspace_binding.is_none());
    assert!(sd.path().join("result.txt").is_file());
    assert_eq!(
        sd.binding_opt().unwrap().unwrap().workdir,
        sd.path().canonicalize().unwrap().display().to_string()
    );
}

#[tokio::test]
async fn external_workspace_is_frozen_and_artifacts_use_its_directory() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let reference = env.workspace(&directory).await;
    let mut spec = work_spec("produce a durable report");
    spec.workspace = Some(reference.clone());
    spec.artifact_id = Some("report-artifact".into());
    spec.prompt.llm_context =
        json!({"tools":{"enabled":true,"tools":[{"groupname":"bash"},{"name":"report"}]}});
    let sd = env.create_work(spec).await;
    let binding = sd.config().unwrap().workspace_binding.unwrap();
    assert_eq!(binding.workspace_id, reference.workspace_id);
    assert_eq!(
        binding.location.directory,
        directory.canonicalize().unwrap()
    );
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call(
            "write",
            "shell",
            json!({"command":"echo durable > result.txt"}),
        ),
        _ => tool_call(
            "deliver",
            "report",
            json!({"report":"done", "artifacts":["result.txt"], "is_end":true}),
        ),
    });
    let result = drive(&sd, &env.deps(llm), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
    assert_eq!(
        sd.binding_opt().unwrap().unwrap().workspace,
        Some(binding.clone())
    );
    assert!(!sd.path().join("result.txt").exists());
    let report = sd.state().unwrap().final_report.unwrap();
    assert_eq!(
        std::fs::read_to_string(sd.path().join(&report.artifacts[0].reference)).unwrap(),
        "durable\n"
    );
    let version = env
        .agent()
        .artifacts()
        .version("report-artifact", &format!("v-{}", sd.sid()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        version.workspace_ref,
        serde_json::to_value(binding).unwrap()
    );
}

#[tokio::test]
async fn conflicting_workdir_is_rejected_at_creation() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = work_spec("wrong cwd");
    spec.workspace = Some(env.workspace(&directory).await);
    spec.prompt.llm_context["runtime"] = json!({"kind":"native", "workdir":env.root});
    let error = create_session(
        &env.app_dir,
        spec,
        env.agent().as_ref(),
        APP,
        env.channels().as_ref(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("workdir conflicts"), "{error}");
}

#[tokio::test]
async fn missing_workspace_fails_permanently_without_recreating_it() {
    let env = Env::new();
    let directory = env.root.join("project");
    let moved = env.root.join("moved");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = work_spec("workspace must stay available");
    spec.workspace = Some(env.workspace(&directory).await);
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({"n":2}),
    };
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| text("first turn"));
    assert!(matches!(
        drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await,
        DriveResult::Idle { .. }
    ));
    assert_eq!(llm.count(), 1);
    std::fs::rename(&directory, &moved).unwrap();
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(
        matches!(
            result,
            DriveResult::Finished {
                outcome: Some(Outcome::Failed),
                ..
            }
        ),
        "{result:?}"
    );
    assert!(!directory.exists());
    assert_eq!(
        sd.state().unwrap().last_error.unwrap()["kind"],
        "workspace_binding_invalid"
    );
    let detail = sd.state().unwrap().last_error.unwrap();
    assert_eq!(detail["file_change_audit"], "incomplete");
    assert_eq!(detail["rollback"], "unsupported");
    assert!(detail["workspace_binding"].is_object());
    assert!(detail["unresolved_tasks"].is_array());
    assert!(detail["unresolved_calls"].is_array());
    assert!(sd
        .report()
        .unwrap()
        .contains("Side effects have not been rolled back"));
    std::fs::rename(&moved, &directory).unwrap();
    assert!(matches!(
        drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await,
        DriveResult::Finished {
            outcome: Some(Outcome::Failed),
            ..
        }
    ));
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn changed_identity_between_calls_prevents_the_next_side_effect() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = work_spec("validate before each call");
    spec.workspace = Some(env.workspace(&directory).await);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| {
        batch(&[
            (
                "remove-identity",
                json!({"command":"mv .opendan-workspace.json identity.saved"}),
            ),
            ("must-not-run", json!({"command":"touch forbidden"})),
        ])
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(
        matches!(
            result,
            DriveResult::Finished {
                outcome: Some(Outcome::Failed),
                ..
            }
        ),
        "{result:?}"
    );
    assert!(directory.join("identity.saved").exists());
    assert!(!directory.join("forbidden").exists());
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn concurrent_writers_and_unsettled_background_work_are_refused() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let reference = env.workspace(&directory).await;
    let mut spec = work_spec("first writer");
    spec.workspace = Some(reference.clone());
    let first = env.create_work(spec).await;
    let mut spec = work_spec("second writer");
    spec.workspace = Some(reference);
    let second = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call(
            "hold",
            "shell",
            json!({"command":"touch started; sleep 0.5"}),
        ),
        _ => text("done"),
    });
    let sd = first.clone();
    let deps = env.deps(llm);
    let running = tokio::spawn(async move { drive(&sd, &deps, StopWhen::Finished).await });
    for _ in 0..100 {
        if directory.join("started").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(directory.join("started").exists());
    let next = ScriptedLlm::new(|_, _| text("second done"));
    assert!(matches!(
        drive(&second, &env.deps(next.clone()), StopWhen::Finished).await,
        DriveResult::Busy { .. }
    ));
    assert_eq!(next.count(), 0);
    assert!(running.await.unwrap().is_finished());
    let lease = first
        .acquire(env.deps(next.clone()).holder())
        .unwrap()
        .into_result("session")
        .unwrap();
    let mut session = first.load(&lease).unwrap();
    session
        .state
        .watched_tasks
        .push("external-task-unconfirmed".into());
    session.commit_state(&lease).unwrap();
    drop(lease);
    assert!(matches!(
        drive(&second, &env.deps(next.clone()), StopWhen::Finished).await,
        DriveResult::Busy { .. }
    ));
    assert_eq!(next.count(), 0);
}

#[tokio::test]
async fn child_inheritance_and_config_updates_cannot_expand_the_binding() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let reference = env.workspace(&directory).await;
    let mut spec = work_spec("readonly parent");
    spec.workspace = Some(WorkspaceRef {
        access: WorkspaceAccess::ReadOnly,
        ..reference
    });
    let parent = env.create_work(spec).await;
    let child = create_sub_session(
        env.agent().as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "child".into(),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(
        parent.config().unwrap().workspace,
        child.config().unwrap().workspace
    );
    let err = create_sub_session(
        env.agent().as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "expanded child".into(),
            workspace: SubWorkspace::Id("another-workspace".into()),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("different workspace"));
    let llm = ScriptedLlm::new(|_, _| text("must not execute without read-only isolation"));
    assert!(matches!(
        drive(&child, &env.deps(llm.clone()), StopWhen::Finished).await,
        DriveResult::Finished {
            outcome: Some(Outcome::Failed),
            ..
        }
    ));
    assert_eq!(llm.count(), 0);
    let lease = parent
        .acquire(env.deps(llm).holder())
        .unwrap()
        .into_result("session")
        .unwrap();
    let mut session = parent.load(&lease).unwrap();
    session.config.workspace = None;
    assert!(session.write_config(&lease).is_err());
    assert!(parent.config().unwrap().workspace.is_some());
}

#[tokio::test]
async fn a_workspace_on_another_host_never_uses_a_local_path_with_the_same_name() {
    let env = Env::new();
    let directory = env.root.join("project");
    std::fs::create_dir(&directory).unwrap();
    let reference = env.workspace(&directory).await;
    let mut spec = work_spec("wrong host");
    spec.workspace = Some(reference.clone());
    let sd = env.create_work(spec).await;
    let entry = libopendan::state::AgentLayout::new(env.agent_root.clone())
        .workspace_entry(&reference.workspace_id);
    let mut record: WorkspaceRecord = libopendan::fsutil::read_json(&entry).unwrap();
    record.runtime_host = "another-host".into();
    libopendan::fsutil::atomic_replace_json(&entry, &record).unwrap();
    let llm = ScriptedLlm::new(|_, _| {
        tool_call(
            "must-not-run",
            "shell",
            json!({"command":"touch forbidden"}),
        )
    });
    assert!(matches!(
        drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await,
        DriveResult::Finished {
            outcome: Some(Outcome::Failed),
            ..
        }
    ));
    assert_eq!(llm.count(), 0);
    assert!(!directory.join("forbidden").exists());
}

#[tokio::test]
async fn an_unbound_ui_session_can_select_a_registered_workspace_for_its_child() {
    let env = Env::new();
    let directory = env.root.join("selected-project");
    std::fs::create_dir(&directory).unwrap();
    let reference = env.workspace(&directory).await;
    let mut spec = work_spec("route user work");
    spec.kind = SessionKind::Ui;
    spec.class = "ui".into();
    let parent = env.create_work(spec).await;
    let agent = env.agent();
    let channels = env.channels();
    let child = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "selected project".into(),
            workspace: SubWorkspace::Id(reference.workspace_id.clone()),
            ..Default::default()
        },
        channels.as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(child.config().unwrap().workspace, Some(reference));
    assert_eq!(
        child
            .config()
            .unwrap()
            .workspace_binding
            .unwrap()
            .location
            .directory,
        directory.canonicalize().unwrap()
    );
    let unknown = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "unknown project".into(),
            workspace: SubWorkspace::Id("ws-00000000-0000-4000-8000-000000000000".into()),
            ..Default::default()
        },
        channels.as_ref(),
    )
    .await
    .unwrap_err();
    assert!(unknown.to_string().contains("not found"), "{unknown}");
}
