mod common;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, CallingConventions, SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use common::*;
use libopendan::api::{InputChannel, SessionSpec};
use libopendan::lock::Acquire;
use libopendan::protocol::*;
use libopendan::runner::history::read_window;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::{SessionDir, SessionTemplate};
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn report_spec(objective: &str) -> SessionSpec {
    let mut spec = work_spec(objective);
    spec.prompt.llm_context = json!({
        "tools": {
            "enabled": true,
            "tools": [{ "groupname": "bash" }, { "name": "report" }]
        }
    });
    spec
}

fn batch(calls: &[(&str, &str, Value)]) -> AiResponse {
    AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        calls
            .iter()
            .map(|(id, name, args)| {
                AiContent::tool_use(
                    *id,
                    *name,
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

fn results(sd: &SessionDir) -> Vec<(String, String, String)> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|entry| match entry.body {
            WorklogBody::ActionResult {
                call_id,
                status,
                result,
                ..
            } => Some((call_id, status, result)),
            _ => None,
        })
        .collect()
}

fn calls(sd: &SessionDir) -> Vec<ActionEntry> {
    read_worklog(sd)
        .into_iter()
        .flat_map(|entry| match entry.body {
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls,
            WorklogBody::Step { actions, .. } => actions,
            _ => Vec::new(),
        })
        .collect()
}

fn deliveries(sd: &SessionDir, report: &str) -> usize {
    read_worklog(sd)
        .iter()
        .filter(|entry| {
            matches!(&entry.body, WorklogBody::ReportDelivery { assistant, .. }
                if assistant.contains(report))
        })
        .count()
}

fn assert_one_completion(sd: &SessionDir, report: &str, rounds: u64) {
    let state = sd.state().unwrap();
    assert_eq!(state.run_state, RunState::Finished);
    assert_eq!(state.outcome, Some(Outcome::Succeeded));
    assert_eq!(state.acceptance, Acceptance::Pending);
    assert_eq!((state.turn_seq, state.turns_completed), (1, 1));
    assert!(state.open_turn.is_none());
    assert!(sd.report().unwrap().contains(report));
    assert_eq!(deliveries(sd, report), 1);
    assert_eq!(sd.statistics().unwrap().rounds, rounds);
    let entries = read_worklog(sd);
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.body.kind() == "turn_ended")
            .count(),
        1
    );
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry.seq, index as u64 + 1);
    }
    assert_eq!(state.worklog.committed_seq, entries.len() as u64);
    let Acquire::Acquired(lease) = sd
        .acquire(HolderInfo {
            runner_id: "report-history-test".into(),
            principal: APP.into(),
            host: None,
            pid: std::process::id(),
            runtime_id: None,
        })
        .unwrap()
    else {
        panic!("finished session is still held");
    };
    let session = sd.load(&lease).unwrap();
    let history = read_window(&session, &session.summary().unwrap(), 100_000)
        .unwrap()
        .lines
        .join("\n");
    assert_eq!(history.matches(report).count(), 1, "{history}");
}

#[tokio::test]
async fn stage_reports_continue_the_batch_and_only_final_report_delivers() {
    let env = Env::new();
    let sd = env
        .create_work(report_spec("submit progress and then final delivery"))
        .await;
    let session_path = sd.path().to_path_buf();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            assert!(req.tool_specs.iter().any(|tool| tool.name == "report"));
            batch(&[
                (
                    "stage-default",
                    "report",
                    json!({ "report": "progress one" }),
                ),
                (
                    "stage-false",
                    "report",
                    json!({ "report": "progress two", "is_end": false }),
                ),
                (
                    "continued",
                    "shell",
                    json!({ "command": "echo continued > progress.marker" }),
                ),
            ])
        }
        1 => {
            let state = SessionDir::open(&session_path).unwrap().state().unwrap();
            assert_eq!(state.latest_report.unwrap().report, "progress two");
            assert!(state.final_report.is_none());
            for id in ["stage-default", "stage-false", "continued"] {
                assert!(
                    has_tool_result(req, id).is_some(),
                    "missing result for {id}"
                );
            }
            tool_call(
                "final",
                "report",
                json!({ "report": "final delivery", "is_end": true }),
            )
        }
        _ => panic!("accepted report must finish without another inference: {n}"),
    });
    let deps = env.deps(llm.clone());
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(llm.count(), 2);
    assert_eq!(
        std::fs::read_to_string(sd.path().join("progress.marker")).unwrap(),
        "continued\n"
    );
    assert_eq!(deliveries(&sd, "progress one"), 0);
    assert_eq!(deliveries(&sd, "progress two"), 0);
    assert_eq!(
        results(&sd)
            .iter()
            .filter(|(_, status, _)| status == "ok")
            .count(),
        4
    );
    assert_one_completion(&sd, "final delivery", 2);
    let entries = read_worklog(&sd);
    assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(read_worklog(&sd), entries);
    assert_eq!(llm.count(), 2);
    assert!(
        libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg("more"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn accepted_final_report_preserves_prior_effects_and_pairs_skipped_calls() {
    let env = Env::new();
    let sd = env
        .create_work(report_spec("finish in the middle of a tool batch"))
        .await;
    let llm = ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0, "a final report does not need an ack round");
        batch(&[
            (
                "before",
                "shell",
                json!({ "command": "echo before >> effects.marker" }),
            ),
            (
                "final",
                "report",
                json!({ "report": "delivered in one round", "is_end": true }),
            ),
            (
                "after",
                "shell",
                json!({ "command": "echo after >> effects.marker" }),
            ),
        ])
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(sd.path().join("effects.marker")).unwrap(),
        "before\n"
    );
    let paired = results(&sd);
    assert_eq!(paired.len(), 3, "{paired:?}");
    for id in ["before", "final", "after"] {
        assert_eq!(
            paired
                .iter()
                .filter(|(call_id, _, _)| call_id == id)
                .count(),
            1
        );
    }
    assert_eq!(
        paired.iter().find(|(id, _, _)| id == "after").unwrap().1,
        "cancelled"
    );
    let state = sd.state().unwrap();
    let (_, snapshot) = sd
        .runs()
        .load_checked(state.last_run.as_ref().unwrap())
        .unwrap();
    let snapshot = snapshot.unwrap();
    assert!(snapshot.state.pending_calls().is_empty());
    assert_one_completion(&sd, "delivered in one round", 1);
}

#[tokio::test]
async fn rejected_reports_are_correctable_and_do_not_skip_sibling_tools() {
    for args in [
        json!({ "is_end": true }),
        json!({ "report": "", "is_end": true }),
        json!({ "report": "invalid bool", "is_end": "true" }),
        json!({ "report": "missing artifact", "artifacts": ["missing.txt"], "is_end": true }),
        json!({ "report": "escaping artifact", "artifacts": ["../outside.txt"], "is_end": true }),
    ] {
        let env = Env::new();
        let sd = env
            .create_work(report_spec("correct a rejected final report"))
            .await;
        let llm = ScriptedLlm::new(move |req, n| match n {
            0 => batch(&[
                ("rejected", "report", args.clone()),
                (
                    "sibling",
                    "shell",
                    json!({ "command": "echo ran > sibling.marker" }),
                ),
            ]),
            1 => {
                assert!(req.messages.iter().any(|m| m.content.iter().any(|c| {
                    matches!(c, AiContent::ToolResult { call_id, is_error: true, .. } if call_id == "rejected")
                })), "{}", render(&req.messages));
                assert!(has_tool_result(req, "sibling").is_some());
                tool_call(
                    "fixed",
                    "report",
                    json!({ "report": "corrected delivery", "is_end": true }),
                )
            }
            _ => panic!("unexpected inference after corrected report: {n}"),
        });
        let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
        assert!(result.is_finished(), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(sd.path().join("sibling.marker")).unwrap(),
            "ran\n"
        );
        assert_eq!(
            results(&sd)
                .iter()
                .find(|(id, _, _)| id == "rejected")
                .unwrap()
                .1,
            "error"
        );
        assert_one_completion(&sd, "corrected delivery", 2);
    }
}

#[tokio::test]
async fn final_report_from_sub_context_returns_to_its_parent() {
    let env = Env::new();
    let mut spec = report_spec("delegate and deliver the parent result");
    spec.prompt.llm_context["tools"]["tools"] = json!([
        { "groupname": "bash" }, { "name": "report" }, { "name": "call_behavior" }
    ]);
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": {
        "research": { "mode": "create_sub_context", "prompt": { "system": "research only" },
            "llm_context": { "tools": { "enabled": true, "tools": [{ "name": "report" }] } } }
    } }),
    );
    let sd = env.create_work(spec).await;
    let session_path = sd.path().to_path_buf();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => batch(&[
            (
                "parent-stage",
                "report",
                json!({ "report": "parent progress before delegation" }),
            ),
            (
                "child",
                "call_behavior",
                json!({ "behavior": "research", "task": "find the number" }),
            ),
            (
                "parent-after",
                "shell",
                json!({ "command": "echo resumed > parent.marker" }),
            ),
        ]),
        1 => tool_call(
            "child-final",
            "report",
            json!({ "report": "child found 42", "result": { "answer": 42 }, "is_end": true }),
        ),
        2 => {
            let state = SessionDir::open(&session_path).unwrap().state().unwrap();
            assert_eq!(
                state.latest_report.unwrap().report,
                "parent progress before delegation"
            );
            assert!(state.final_report.is_none());
            let child: Value =
                serde_json::from_str(&has_tool_result(req, "child").unwrap()).unwrap();
            assert_eq!(child["submission"]["report"], "child found 42");
            assert_eq!(child["submission"]["result"], json!({ "answer": 42 }));
            assert!(has_tool_result(req, "parent-after").is_some());
            assert!(has_tool_result(req, "child-final").is_none());
            tool_call(
                "parent-final",
                "report",
                json!({ "report": "parent approved 42", "is_end": true }),
            )
        }
        _ => panic!("unexpected inference {n}"),
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(sd.path().join("parent.marker")).unwrap(),
        "resumed\n"
    );
    assert!(sd.state().unwrap().process_stack.is_empty());
    assert_eq!(sd.statistics().unwrap().runs, 2);
    assert_one_completion(&sd, "parent approved 42", 3);
}

#[tokio::test]
async fn behavior_stage_report_continues_until_an_explicit_final_report() {
    let env = Env::new();
    let mut spec = report_spec("behavior report intent");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    let sd = env.create_work(spec).await;
    let session_path = sd.path().to_path_buf();
    let llm = ScriptedLlm::new(move |_, n| match n {
        0 => text("<response><report>stage without end</report></response>"),
        1 => {
            let state = SessionDir::open(&session_path).unwrap().state().unwrap();
            assert_eq!(state.latest_report.unwrap().report, "stage without end");
            assert!(state.final_report.is_none());
            text("<response><report end=\"false\">stage with false</report></response>")
        }
        2 => {
            let state = SessionDir::open(&session_path).unwrap().state().unwrap();
            assert_eq!(state.latest_report.unwrap().report, "stage with false");
            text("<response><report end=\"true\">behavior delivery</report></response>")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(deliveries(&sd, "stage without end"), 0);
    assert_eq!(deliveries(&sd, "stage with false"), 0);
    assert_one_completion(&sd, "behavior delivery", 3);
}

#[tokio::test]
async fn behavior_native_progress_report_keeps_its_call_and_receipt_after_xml_completion() {
    let env = Env::new();
    let mut spec = report_spec("keep native progress audit inside a behavior step");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    spec.prompt.llm_context["tools"]["tools2actions"] = json!(false);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call(
            "native-progress",
            "report",
            json!({ "report": "native progress checkpoint" }),
        ),
        1 => {
            assert!(has_tool_result(req, "native-progress").is_some());
            text("<response><report end=\"true\">native progress verified</report></response>")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    let submitted = calls(&sd);
    assert_eq!(
        submitted
            .iter()
            .filter(|c| c.call_id == "native-progress")
            .count(),
        1
    );
    let progress = submitted
        .iter()
        .find(|c| c.call_id == "native-progress")
        .unwrap();
    assert_eq!(progress.tool, "report");
    assert_eq!(
        progress.args,
        json!({ "report": "native progress checkpoint" })
    );
    let paired = results(&sd);
    assert_eq!(paired.len(), 1, "{paired:?}");
    assert_eq!(
        (paired[0].0.as_str(), paired[0].1.as_str()),
        ("native-progress", "ok")
    );
    assert_eq!(deliveries(&sd, "native progress checkpoint"), 0);
    assert_one_completion(&sd, "native progress verified", 2);
}

#[tokio::test]
async fn behavior_native_final_report_keeps_every_call_and_pairs_the_skipped_sibling() {
    let env = Env::new();
    let mut spec = report_spec("audit native final completion inside a behavior loop");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    spec.prompt.llm_context["tools"]["tools2actions"] = json!(false);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0, "a native final report needs no XML acknowledgment");
        batch(&[
            (
                "native-before",
                "shell",
                json!({ "command": "echo before >> native.marker" }),
            ),
            (
                "native-final",
                "report",
                json!({ "report": "native behavior delivery", "is_end": true }),
            ),
            (
                "native-after",
                "shell",
                json!({ "command": "echo after >> native.marker" }),
            ),
        ])
    });
    let deps = env.deps(llm.clone());
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(sd.path().join("native.marker")).unwrap(),
        "before\n"
    );
    let submitted = calls(&sd);
    let paired = results(&sd);
    assert_eq!(submitted.len(), 3, "{submitted:?}");
    assert_eq!(paired.len(), 3, "{paired:?}");
    for (id, status) in [
        ("native-before", "ok"),
        ("native-final", "ok"),
        ("native-after", "cancelled"),
    ] {
        assert_eq!(submitted.iter().filter(|c| c.call_id == id).count(), 1);
        let matching: Vec<_> = paired
            .iter()
            .filter(|(call_id, _, _)| call_id == id)
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].1, status);
    }
    assert_one_completion(&sd, "native behavior delivery", 1);
    let entries = read_worklog(&sd);
    assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(read_worklog(&sd), entries);
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn behavior_stage_artifacts_capture_the_file_before_same_step_actions_change_it() {
    let env = Env::new();
    let mut spec = report_spec("preserve the artifact submitted before a rewrite");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    spec.prompt.llm_context["tools"]["tools2actions"] = json!(true);
    let sd = env.create_work(spec).await;
    std::fs::write(sd.path().join("source.txt"), "submitted content\n").unwrap();
    let session_path = sd.path().to_path_buf();
    let llm = ScriptedLlm::new(move |_, n| {
        match n {
        0 => text("<response><report>stage artifact before rewrite</report><artifacts>[\"source.txt\"]</artifacts><actions><shell><![CDATA[echo changed content > source.txt]]></shell></actions></response>"),
        1 => {
            let state = SessionDir::open(&session_path).unwrap().state().unwrap();
            let stage = state.latest_report.unwrap();
            assert_eq!(stage.report, "stage artifact before rewrite");
            assert!(!stage.is_end);
            assert_eq!(stage.artifacts.len(), 1);
            assert_eq!(std::fs::read_to_string(session_path.join("source.txt")).unwrap(), "changed content\n");
            assert_eq!(std::fs::read_to_string(session_path.join(&stage.artifacts[0].reference)).unwrap(), "submitted content\n");
            text("<response><report end=\"true\">rewrite completed</report></response>")
        }
        _ => panic!("unexpected inference {n}"),
    }
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(deliveries(&sd, "stage artifact before rewrite"), 0);
    let state = sd.state().unwrap();
    let record = sd.runs().record(state.last_run.as_ref().unwrap()).unwrap();
    let reports = record.host.as_ref().unwrap().extra["reports"]
        .as_array()
        .unwrap();
    assert_eq!(
        reports
            .iter()
            .filter(|r| r["report"] == "stage artifact before rewrite")
            .count(),
        1
    );
    assert_one_completion(&sd, "rewrite completed", 2);
}

#[tokio::test]
async fn conflicting_behavior_end_report_does_not_execute_actions_or_publish_report() {
    let env = Env::new();
    let mut spec = report_spec("reject an ending decision with actions");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    spec.prompt.llm_context["tools"]["tools2actions"] = json!(true);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        match n {
        0 => text("<response><report end=\"true\">must not publish</report><actions><shell><![CDATA[echo wrong > forbidden.marker]]></shell></actions></response>"),
        1 => {
            assert!(!req.messages.is_empty());
            text("<response><report end=\"true\">valid behavior delivery</report></response>")
        }
        _ => panic!("unexpected inference {n}"),
    }
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert!(!sd.path().join("forbidden.marker").exists());
    assert!(!sd.report().unwrap().contains("must not publish"));
    assert_eq!(deliveries(&sd, "must not publish"), 0);
    assert_one_completion(&sd, "valid behavior delivery", 2);
}

#[tokio::test]
async fn invalid_behavior_artifact_is_reported_back_for_correction() {
    let env = Env::new();
    let mut spec = report_spec("correct a behavior artifact");
    spec.prompt.llm_context["loop_model"] = json!("behavior");
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        match n {
        0 => text("<response><report end=\"true\">invalid artifact delivery</report><artifacts>[\"missing.txt\"]</artifacts></response>"),
        1 => {
            let transcript = render(&req.messages);
            assert!(transcript.contains("missing.txt"), "{transcript}");
            text("<response><report end=\"true\">corrected behavior artifact</report></response>")
        }
        _ => panic!("unexpected inference {n}"),
    }
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_one_completion(&sd, "corrected behavior artifact", 2);
    assert_eq!(deliveries(&sd, "invalid artifact delivery"), 0);
}

#[tokio::test]
async fn explicit_completion_without_a_queue_fails_when_the_model_omits_report() {
    let env = Env::new();
    let mut spec = report_spec("must explicitly deliver");
    spec.policy = serde_json::from_value(json!({ "completion": "explicit_report" })).unwrap();
    spec.input_channel = Some(InputChannel::None);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0);
        text("ordinary assistant text")
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Failed));
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn explicit_completion_ui_waits_after_done_and_finishes_after_report() {
    let env = Env::new();
    let mut spec = SessionTemplate::load("ui", None)
        .unwrap()
        .spec("chat with explicit delivery");
    spec.route_key = Some(format!("{AGENT}/dm%3A{USER}"));
    spec.prompt.llm_context = report_spec("").prompt.llm_context;
    spec.policy = serde_json::from_value(json!({ "completion": "explicit_report" })).unwrap();
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => text("ordinary conversational reply"),
        1 => tool_call(
            "final",
            "report",
            json!({ "report": "final UI delivery", "is_end": true }),
        ),
        _ => panic!("unexpected inference {n}"),
    });
    let deps = env.deps(llm.clone());
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg("first message"))
        .await
        .unwrap();
    let first = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(first, DriveResult::TurnClosed { .. }), "{first:?}");
    assert_ne!(sd.state().unwrap().run_state, RunState::Finished);
    assert_eq!(sd.state().unwrap().turns_completed, 1);
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg("please finish"))
        .await
        .unwrap();
    let last = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(last.is_finished(), "{last:?}");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
    assert_eq!(sd.state().unwrap().turns_completed, 2);
    assert_eq!(deliveries(&sd, "final UI delivery"), 1);
    assert_eq!(sd.statistics().unwrap().rounds, 2);
}

#[tokio::test]
async fn final_artifacts_are_stable_and_share_the_submitted_result_with_delivery() {
    let env = Env::new();
    let sd = env
        .create_work(report_spec("deliver one selected artifact"))
        .await;
    std::fs::write(sd.path().join("answer.txt"), "verified answer\n").unwrap();
    std::fs::write(sd.path().join("private.txt"), "not selected\n").unwrap();
    let llm = ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0);
        tool_call(
            "final",
            "report",
            json!({
                "report": "artifact delivery",
                "artifacts": ["answer.txt"],
                "result": { "answer": 42, "checks": [true, "passed"] },
                "is_end": true
            }),
        )
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_one_completion(&sd, "artifact delivery", 1);
    let state = sd.state().unwrap();
    let submitted = state.final_report.as_ref().unwrap();
    assert_eq!(state.latest_report.as_ref(), Some(submitted));
    assert_eq!(
        submitted.source,
        ReportSource::Tool {
            call_id: "final".into()
        }
    );
    assert_eq!(
        submitted.result,
        Some(json!({ "answer": 42, "checks": [true, "passed"] }))
    );
    assert_eq!(submitted.artifacts.len(), 1);
    let artifact = &submitted.artifacts[0];
    assert_eq!(artifact.path, "answer.txt");
    assert!(!artifact.digest.is_empty());
    let stable_path = sd.path().join(&artifact.reference);
    assert_eq!(
        std::fs::read_to_string(&stable_path).unwrap(),
        "verified answer\n"
    );
    std::fs::write(sd.path().join("answer.txt"), "changed source\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(&stable_path).unwrap(),
        "verified answer\n"
    );
    let delivered = read_worklog(&sd)
        .into_iter()
        .find_map(|entry| match entry.body {
            WorklogBody::ReportDelivery {
                submission,
                assistant,
                ..
            } => Some((submission, assistant)),
            _ => None,
        })
        .unwrap();
    assert_eq!(&delivered.0, submitted);
    assert_eq!(delivered.1, submitted.delivery_text());
    assert!(sd.report().unwrap().contains(&submitted.delivery_text()));
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_artifacts_cannot_escape_the_bound_workspace() {
    let env = Env::new();
    let sd = env
        .create_work(report_spec("reject an escaping symlink"))
        .await;
    let outside = env.root.join("outside.txt");
    std::fs::write(&outside, "not an authorized artifact\n").unwrap();
    std::os::unix::fs::symlink(&outside, sd.path().join("escape.txt")).unwrap();
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call(
            "escape",
            "report",
            json!({ "report": "escaped", "artifacts": ["escape.txt"], "is_end": true }),
        ),
        1 => {
            assert!(has_tool_result(req, "escape").is_some());
            tool_call(
                "fixed",
                "report",
                json!({ "report": "safe delivery", "is_end": true }),
            )
        }
        _ => panic!("unexpected inference {n}"),
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        results(&sd)
            .iter()
            .find(|(id, _, _)| id == "escape")
            .unwrap()
            .1,
        "error"
    );
    assert!(sd
        .state()
        .unwrap()
        .final_report
        .unwrap()
        .artifacts
        .is_empty());
    assert_one_completion(&sd, "safe delivery", 2);
}

#[tokio::test]
async fn repeated_report_call_is_idempotent_and_changed_arguments_are_rejected() {
    let env = Env::new();
    let sd = env
        .create_work(report_spec("repeat a report request"))
        .await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 | 1 => tool_call("stage", "report", json!({ "report": "one stable stage" })),
        2 => tool_call("stage", "report", json!({ "report": "changed stage" })),
        3 => {
            let newest = req
                .messages
                .iter()
                .rev()
                .flat_map(|m| m.content.iter().rev())
                .find(|c| matches!(c, AiContent::ToolResult { call_id, .. } if call_id == "stage"))
                .unwrap();
            assert!(matches!(
                newest,
                AiContent::ToolResult { is_error: true, .. }
            ));
            tool_call(
                "final",
                "report",
                json!({ "report": "repeat-safe delivery", "is_end": true }),
            )
        }
        _ => panic!("unexpected inference {n}"),
    });
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    let state = sd.state().unwrap();
    let record = sd.runs().record(state.last_run.as_ref().unwrap()).unwrap();
    let reports = record.host.as_ref().unwrap().extra["reports"]
        .as_array()
        .unwrap();
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert_eq!(
        reports
            .iter()
            .filter(|r| r["report"] == "one stable stage")
            .count(),
        1
    );
    assert!(!reports.iter().any(|r| r["report"] == "changed stage"));
    assert_one_completion(&sd, "repeat-safe delivery", 4);
}

struct BackgroundTask;

#[async_trait]
impl AgentTool for BackgroundTask {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "test_background_task".into(),
            description: "start a test task or observe its completed result".into(),
            args_schema: json!({ "type": "object", "properties": { "finish": { "type": "boolean" } } }),
            output_schema: json!({ "type": "object" }),
            usage: None,
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let finished = args["finish"].as_bool().unwrap_or(false);
        let mut result = AgentToolResult::from_details(json!({ "finished": finished }))
            .with_tool("test_background_task")
            .with_task_id("test:report-task");
        result.summary = if finished {
            "task finished"
        } else {
            "task started"
        }
        .into();
        if finished {
            result.return_code = Some(0);
        }
        Ok(result)
    }
}

#[tokio::test]
async fn final_report_requires_background_tasks_to_be_settled() {
    let env = Env::new();
    let mut spec = report_spec("settle tasks before completion");
    spec.prompt.llm_context["tools"]["tools"] = json!([
        { "name": "report" }, { "name": "test_background_task" }
    ]);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("start", "test_background_task", json!({})),
        1 => tool_call(
            "premature",
            "report",
            json!({ "report": "premature delivery", "is_end": true }),
        ),
        2 => {
            assert!(has_tool_result(req, "premature")
                .unwrap()
                .contains("active"));
            tool_call("settled", "test_background_task", json!({ "finish": true }))
        }
        3 => tool_call(
            "final",
            "report",
            json!({ "report": "settled delivery", "is_end": true }),
        ),
        _ => panic!("unexpected inference {n}"),
    });
    let mut deps = env.deps(llm.clone());
    deps.xllm.host_tools.insert(
        "test_background_task".into(),
        std::sync::Arc::new(BackgroundTask),
    );
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(
        results(&sd)
            .iter()
            .find(|(id, _, _)| id == "premature")
            .unwrap()
            .1,
        "error"
    );
    assert_eq!(deliveries(&sd, "premature delivery"), 0);
    assert_one_completion(&sd, "settled delivery", 4);
}

#[tokio::test]
async fn behavior_final_report_requires_background_tasks_to_be_settled() {
    let env = Env::new();
    let mut spec = report_spec("settle tasks before behavior completion");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true, "tools": [{ "name": "test_background_task" }] }
    });
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("<response><actions><test_background_task/></actions></response>"),
        1 => text("<response><report end=\"true\">premature behavior delivery</report></response>"),
        2 => {
            let transcript = render(&req.messages);
            assert!(transcript.contains("active"), "{transcript}");
            text("<response><actions><test_background_task finish=\"true\"/></actions></response>")
        }
        3 => text("<response><report end=\"true\">settled behavior delivery</report></response>"),
        _ => panic!("unexpected inference {n}"),
    });
    let mut deps = env.deps(llm.clone());
    deps.xllm.host_tools.insert(
        "test_background_task".into(),
        std::sync::Arc::new(BackgroundTask),
    );
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(deliveries(&sd, "premature behavior delivery"), 0);
    assert_one_completion(&sd, "settled behavior delivery", 4);
}

const CRASH_ROOT: &str = "LIBOPENDAN_REPORT_TEST_ROOT";
const CRASH_SESSION: &str = "LIBOPENDAN_REPORT_TEST_SESSION";

fn crashing_report_script(behavior: bool) -> std::sync::Arc<ScriptedLlm> {
    ScriptedLlm::new(move |_, n| {
        if behavior {
            return match n {
                0 => text("<response><actions><shell><![CDATA[echo once >> crash.marker]]></shell></actions></response>"),
                1 => text("<response><report end=\"true\">crash-safe delivery</report><artifacts>[\"answer.txt\"]</artifacts><result>null</result></response>"),
                _ => panic!("unexpected inference {n}"),
            };
        }
        assert_eq!(n, 0);
        batch(&[
            (
                "before",
                "shell",
                json!({ "command": "echo once >> crash.marker" }),
            ),
            (
                "final",
                "report",
                json!({
                    "report": "crash-safe delivery",
                    "artifacts": ["answer.txt"],
                    "result": null,
                    "is_end": true
                }),
            ),
            (
                "after",
                "shell",
                json!({ "command": "echo forbidden >> crash.marker" }),
            ),
        ])
    })
}

#[tokio::test]
async fn report_crash_child_driver() {
    let (Ok(root), Ok(session)) = (std::env::var(CRASH_ROOT), std::env::var(CRASH_SESSION)) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let sd = SessionDir::open(&session).unwrap();
    let xml = sd.config().unwrap().prompt.llm_context["tools"]["tools2actions"] == true;
    let result = drive(
        &sd,
        &env.deps(crashing_report_script(xml)),
        StopWhen::Finished,
    )
    .await;
    assert!(result.is_finished(), "{result:?}");
}

#[tokio::test]
async fn final_report_recovers_without_inference_or_duplicate_delivery_at_commit_windows() {
    for mode in ["function_call", "behavior_xml", "behavior_native"] {
        let behavior = mode != "function_call";
        let xml = mode == "behavior_xml";
        for fault in [
            "report:after_persist",
            "report:after_paired_checkpoint",
            "outcome:after_checkpoint",
            "finish_run:after_flush",
        ] {
            let env = Env::new();
            let mut spec = report_spec("recover a submitted final report");
            if behavior {
                spec.prompt.llm_context["loop_model"] = json!("behavior");
                spec.prompt.llm_context["tools"]["tools2actions"] = json!(xml);
            }
            let sd = env.create_work(spec).await;
            std::fs::write(sd.path().join("answer.txt"), "stable artifact\n").unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "report_crash_child_driver",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CRASH_ROOT, &env.root)
                .env(CRASH_SESSION, sd.path())
                .env("LIBOPENDAN_FAULT", fault)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let started = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > Duration::from_secs(30) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not reach {fault}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            assert!(!status.success(), "fault not reached: {fault}");
            let llm = ScriptedLlm::new(|_, n| {
                panic!("submitted final report must recover without inference: {n}")
            });
            let deps = env.deps(llm.clone());
            let result = drive(&sd, &deps, StopWhen::Finished).await;
            assert!(result.is_finished(), "{mode} {fault}: {result:?}");
            assert_eq!(llm.count(), 0, "{fault}");
            assert_eq!(
                std::fs::read_to_string(sd.path().join("crash.marker")).unwrap(),
                "once\n",
                "{fault}"
            );
            assert_one_completion(&sd, "crash-safe delivery", if xml { 2 } else { 1 });
            let state = sd.state().unwrap();
            let record = sd.runs().record(state.last_run.as_ref().unwrap()).unwrap();
            let reports = record.host.as_ref().unwrap().extra["reports"]
                .as_array()
                .unwrap();
            assert_eq!(reports.len(), 1, "{fault}");
            let final_report = state.final_report.as_ref().unwrap();
            assert_eq!(final_report.result, Some(Value::Null), "{fault}");
            assert_eq!(final_report.artifacts.len(), 1);
            assert_eq!(
                std::fs::read_to_string(sd.path().join(&final_report.artifacts[0].reference))
                    .unwrap(),
                "stable artifact\n"
            );
            let paired = results(&sd);
            assert_eq!(paired.len(), if xml { 1 } else { 3 }, "{fault}: {paired:?}");
            for id in if xml {
                vec![]
            } else {
                vec!["before", "final", "after"]
            } {
                assert_eq!(
                    paired
                        .iter()
                        .filter(|(call_id, _, _)| call_id == id)
                        .count(),
                    1,
                    "{fault}"
                );
                assert_eq!(
                    calls(&sd).iter().filter(|call| call.call_id == id).count(),
                    1,
                    "{mode} {fault}: missing or repeated call {id}"
                );
            }
            let entries = read_worklog(&sd);
            assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
            assert_eq!(read_worklog(&sd), entries, "{fault}");
        }
    }
}
