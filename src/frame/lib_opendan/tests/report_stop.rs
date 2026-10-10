mod common;

use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::SessionDir;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ROOT: &str = "LIBOPENDAN_REPORT_STOP_ROOT";
const SESSION: &str = "LIBOPENDAN_REPORT_STOP_SESSION";
const NESTED: &str = "LIBOPENDAN_REPORT_STOP_NESTED";

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
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                )
            })
            .collect(),
    ))
}

#[tokio::test]
async fn report_stop_crash_driver() {
    let (Ok(root), Ok(session)) = (std::env::var(ROOT), std::env::var(SESSION)) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let session = SessionDir::open(session).unwrap();
    let nested = std::env::var(NESTED).unwrap() == "true";
    let llm = ScriptedLlm::new(move |_, n| {
        if nested && n == 0 {
            return batch(&[
                (
                    "delegate",
                    "call_behavior",
                    json!({"behavior":"research", "task":"report your result"}),
                ),
                (
                    "parent-after",
                    "shell",
                    json!({"command":"echo forbidden > parent-after.marker"}),
                ),
            ]);
        }
        assert_eq!(n, usize::from(nested));
        batch(&[
            (
                "before",
                "shell",
                json!({"command":"echo once >> before.marker"}),
            ),
            (
                "final",
                "report",
                json!({"report":"accepted before stop", "result":null, "is_end":true}),
            ),
            (
                "after",
                "shell",
                json!({"command":"echo forbidden > after.marker"}),
            ),
        ])
    });
    let result = drive(&session, &env.deps(llm), StopWhen::Finished).await;
    panic!("expected report crash, got {result:?}");
}

#[tokio::test]
async fn accepted_final_report_is_delivered_and_paired_before_recovered_stop() {
    for (nested, max_turns) in [(false, false), (false, true), (true, false)] {
        for fault in ["report:after_persist", "report:after_paired_checkpoint"] {
            let env = Env::new();
            let mut spec = work_spec("resolve an accepted report before Stop");
            spec.prompt.llm_context = json!({"tools":{"enabled":true, "tools":[
                {"groupname":"bash"}, {"name":"report"}, {"name":"call_behavior"}
            ]}});
            if nested {
                spec.extensions.insert("opendan".into(), json!({"behaviors":{
                    "research": {"mode":"create_sub_context", "prompt":{"system":"research"},
                    "llm_context":{"tools":{"enabled":true,"tools":[{"groupname":"bash"},{"name":"report"}]}}}
                }}));
            } else {
                spec.prompt.llm_context["tools"]["tools"] = json!([
                    {"groupname":"bash"}, {"name":"report"}
                ]);
                if max_turns {
                    spec.end_condition = EndCondition {
                        kind: EndConditionType::MaxTurns,
                        detail: json!({"n":2}),
                    };
                }
            }
            let session = env.create_work(spec).await;
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "report_stop_crash_driver",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(ROOT, &env.root)
                .env(SESSION, session.path())
                .env(NESTED, nested.to_string())
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
            assert!(!status.success());
            let accepted_run = session.state().unwrap().live_run.unwrap().run_id;
            let accepted_record = session.runs().record(&accepted_run).unwrap();
            let accepted_reports = accepted_record.host.as_ref().unwrap().extra["reports"]
                .as_array()
                .unwrap();
            assert_eq!(accepted_reports.len(), 1);
            assert_eq!(accepted_reports[0]["is_end"], true);
            let accepted_id = accepted_reports[0]["id"].as_str().unwrap();
            let llm = ScriptedLlm::new(|_, n| panic!("Stop/report recovery must not infer: {n}"));
            let deps = env.deps(llm.clone());
            deps.stop.request();
            let result = drive(&session, &deps, StopWhen::Finished).await;
            assert!(
                result.is_finished(),
                "nested={nested} fault={fault}: {result:?}"
            );
            assert_eq!(llm.count(), 0);
            assert_eq!(
                std::fs::read_to_string(session.path().join("before.marker")).unwrap(),
                "once\n"
            );
            assert!(!session.path().join("after.marker").exists());
            assert!(!session.path().join("parent-after.marker").exists());
            let state = session.state().unwrap();
            assert_eq!(
                state.outcome,
                Some(if nested || max_turns {
                    Outcome::Stopped
                } else {
                    Outcome::Succeeded
                })
            );
            assert!(state.process_stack.is_empty());
            assert!(state.open_turn.is_none());
            let entries = read_worklog(&session);
            let results: Vec<_> = entries
                .iter()
                .filter_map(|entry| match &entry.body {
                    WorklogBody::ActionResult {
                        call_id,
                        status,
                        result,
                        ..
                    } => Some((call_id.as_str(), status.as_str(), result.as_str())),
                    _ => None,
                })
                .collect();
            for (id, expected) in [("before", "ok"), ("final", "ok"), ("after", "cancelled")] {
                let matched: Vec<_> = results
                    .iter()
                    .filter(|(call_id, _, _)| *call_id == id)
                    .collect();
                assert_eq!(matched.len(), 1, "{results:?}");
                assert_eq!(matched[0].1, expected, "{results:?}");
                if id == "final" {
                    assert!(matched[0].2.contains(accepted_id), "{results:?}");
                }
            }
            if nested {
                assert!(results.iter().any(|(id, status, result)| *id == "delegate"
                    && *status == "ok"
                    && result.contains("accepted before stop")
                    && result.contains(accepted_id)));
                assert!(results
                    .iter()
                    .any(|(id, status, _)| *id == "parent-after" && *status == "cancelled"));
                assert!(state.final_report.is_none());
            } else {
                assert_eq!(
                    entries
                        .iter()
                        .filter(|entry| matches!(&entry.body, WorklogBody::ReportDelivery { .. }))
                        .count(),
                    1
                );
                assert_eq!(state.final_report.unwrap().result, Some(Value::Null));
            }
            if session.runs().exists(&accepted_run) {
                let record = session.runs().record(&accepted_run).unwrap();
                assert_eq!(
                    record.host.unwrap().extra["reports"],
                    json!(accepted_reports)
                );
            } else {
                assert!(nested, "only the returned child run can be cleaned up");
            }
            let (_, snapshot) = session
                .runs()
                .load_checked(state.last_run.as_ref().unwrap())
                .unwrap();
            assert!(snapshot.unwrap().state.tool_batch.is_none());
        }
    }
}
