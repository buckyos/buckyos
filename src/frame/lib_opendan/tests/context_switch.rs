//! Context scheduling (context switch TODO, V4–V7): tool-triggered sub
//! contexts (`call_behavior`), fork history, hand-over recovery.

mod common;

use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_calls(calls: &[(&str, &str, Value)]) -> AiResponse {
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

/// A function call session whose tool set has `call_behavior`.
fn fc_spec(obj: &str, behaviors: Value) -> libopendan::api::SessionSpec {
    let mut spec = work_spec(obj);
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [ { "groupname": "bash" }, { "name": "call_behavior" } ] }
    });
    spec.extensions
        .insert("opendan".into(), json!({ "behaviors": behaviors }));
    spec
}

fn outcomes(sd: &libopendan::SessionDir) -> Vec<String> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Outcome { kind, .. } => Some(kind),
            _ => None,
        })
        .collect()
}

fn result_ids(sd: &libopendan::SessionDir) -> Vec<String> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect()
}

/// V4 / V5: a fork called from a tool batch. The call returns at once
/// ("registered"), the rest of the batch runs, and the branch starts when
/// the caller's segment ends: it gets the caller's system and complete
/// paired history. Its result comes back to the caller as the input of its
/// hand-over batch, naming the call — nothing runs twice.
#[tokio::test]
async fn tool_triggered_fork_keeps_full_history_and_returns_to_the_call() {
    let env = Env::new();
    let sd = env
        .create_work(fc_spec(
            "fork from a tool batch",
            json!({ "research": { "mode": "fork" } }),
        ))
        .await;
    let parent_system = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let ps = parent_system.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            *ps.lock().unwrap() = system_of(req);
            tool_call("c1", "shell", json!({ "command": "echo pre-fork >> marks.log; echo pre-fork" }))
        }
        1 => tool_calls(&[
            ("f1", "call_behavior", json!({ "behavior": "research", "task": "find X" })),
            ("c2", "shell", json!({ "command": "echo after-fork >> marks.log; echo after-fork" })),
        ]),
        2 => {
            // The caller: the call was registered, the batch ran on.
            assert!(has_tool_result(req, "f1").unwrap().contains("registered"));
            assert!(has_tool_result(req, "c2").unwrap().contains("after-fork"));
            text("waiting for the research")
        }
        3 => {
            // The branch: same system, the caller's complete history.
            assert_eq!(system_of(req), *ps.lock().unwrap());
            assert!(has_tool_result(req, "c1").unwrap().contains("pre-fork"));
            assert!(has_tool_result(req, "c2").unwrap().contains("after-fork"));
            let last = last_user_text(req);
            assert!(last.contains("<sub_task mode=\"fork\"") && last.contains("find X"), "{last}");
            tool_call("r1", "shell", json!({ "command": "echo in-fork" }))
        }
        4 => text("X is 42"),
        5 => {
            // The caller again: the result as its input, naming the call.
            let last = last_user_text(req);
            assert!(last.contains("<process_result") && last.contains("call_id=\"f1\""), "{last}");
            assert!(last.contains("X is 42"), "{last}");
            assert!(has_tool_result(req, "r1").is_none(), "branch history stays in the branch");
            text("final answer")
        }
        _ => panic!("unexpected call {n}"),
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 6);
    assert!(sd.report().unwrap().contains("final answer"));
    assert_eq!(outcomes(&sd), vec!["suspended", "process_done", "done"]);
    // Every call has exactly one result in the worklog; inherited history
    // is not written again by the branch.
    let mut ids = result_ids(&sd);
    ids.sort();
    assert_eq!(ids, vec!["c1", "c2", "f1", "r1"]);
    let marks = std::fs::read_to_string(sd.path().join("marks.log")).unwrap();
    assert_eq!(marks, "pre-fork\nafter-fork\n", "no tool ran twice");
    let st = sd.state().unwrap();
    assert!(st.process_stack.is_empty() && st.process_result.is_none());
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let k = kinds(&read_worklog(&sd));
    assert_eq!(k.iter().filter(|x| **x == "turn_started").count(), 1);
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.runs, stats.turns), (6, 2, 1));
}

/// V3 / V5: create_sub_context through the tool — its own system, no caller
/// history; it starts when the caller stops acting and its result is the
/// caller's next input. A bad call is the model's error to correct.
#[tokio::test]
async fn tool_triggered_sub_context_has_its_own_system() {
    let env = Env::new();
    let sd = env
        .create_work(fc_spec(
            "delegate",
            json!({ "do": { "mode": "create_sub_context", "prompt": { "system": "SYSTEM-DO" },
                            "llm_context": { "tools": { "enabled": true } } } }),
        ))
        .await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("c1", "shell", json!({ "command": "echo parent-fact" })),
        1 => tool_call("d1", "call_behavior", json!({ "behavior": "do", "task": "build it" })),
        2 => {
            assert!(has_tool_result(req, "d1").unwrap().contains("registered"));
            text("waiting for the build")
        }
        3 => {
            assert!(system_of(req).contains("SYSTEM-DO"));
            assert!(has_tool_result(req, "c1").is_none(), "no caller history");
            assert!(last_user_text(req).contains("build it"));
            // The sub context has its own tool set: no `call_behavior`.
            assert!(!req.tool_specs.iter().any(|t| t.name == "call_behavior"));
            text("built")
        }
        4 => {
            let last = last_user_text(req);
            assert!(last.contains("call_id=\"d1\"") && last.contains("built"), "{last}");
            // An unknown target is the model's error to correct.
            tool_call("d2", "call_behavior", json!({ "behavior": "nope", "task": "x" }))
        }
        5 => {
            assert!(has_tool_result(req, "d2").unwrap().contains("cannot be called"));
            text("done")
        }
        _ => panic!("unexpected call {n}"),
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 6);
    assert_eq!(outcomes(&sd), vec!["suspended", "process_done", "done"]);
}

/// Behavior loop: `call_behavior` as an action of a step. The step runs
/// on; the fork starts when the caller yields and sees its completed steps.
#[tokio::test]
async fn action_triggered_fork_returns_into_the_step() {
    let env = Env::new();
    let mut spec = work_spec("fork from a step");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true,
                   "tools": [ { "groupname": "bash" }, { "name": "call_behavior" } ] }
    });
    spec.prompt.behavior = Some("plan".into());
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "research": { "mode": "fork" } } }),
    );
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        let all = render(&req.messages);
        match n {
            0 => text("<response><actions><shell><![CDATA[echo s0-out]]></shell></actions></response>"),
            1 => text("<response><actions><call_behavior behavior=\"research\" task=\"find Y\"/><shell><![CDATA[echo after-call]]></shell></actions></response>"),
            2 => {
                // The call registered the fork; the step ran on. Yield.
                assert!(all.contains("registered") && all.contains("after-call"), "{all}");
                text("<response><report>waiting for the research</report></response>")
            }
            3 => {
                assert!(all.contains("s0-out") && all.contains("after-call"), "completed steps are inherited:\n{all}");
                assert!(all.contains("find Y"), "{all}");
                text("<response><report end=\"true\"><![CDATA[Y is 7]]></report></response>")
            }
            4 => {
                assert!(all.contains("Y is 7"), "{all}");
                text("<response><report end=\"true\"><![CDATA[final]]></report></response>")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 5);
    assert_eq!(outcomes(&sd), vec!["suspended", "process_done", "done"]);
    // Step identity stays unique across caller and branch.
    let mut ids: Vec<(String, u32)> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Step { run_id, step_index, .. } => Some((run_id, step_index)),
            _ => None,
        })
        .collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
    let mut calls = result_ids(&sd);
    let n = calls.len();
    calls.sort();
    calls.dedup();
    assert_eq!(calls.len(), n, "call ids reused: {calls:?}");
}

/// V10: a caller rebuilt from the session history gets the sub context's
/// result, not its transcript.
#[tokio::test]
async fn returned_sub_context_transcript_stays_out_of_rebuilt_history() {
    let env = Env::new();
    let mut spec = fc_spec("two turns", json!({ "research": { "mode": "fork" } }));
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("f1", "call_behavior", json!({ "behavior": "research", "task": "dig" })),
        1 => text("waiting for the research"),
        2 => tool_call("r1", "shell", json!({ "command": "echo CHILD-ONLY-OUTPUT" })),
        3 => text("child result R"),
        4 => text("turn one answer"),
        _ => {
            // A new run of the next Turn, built from the session history.
            let all = render(&req.messages);
            assert!(all.contains("<session_history>"), "{all}");
            assert!(all.contains("child result R"), "{all}");
            assert!(!all.contains("CHILD-ONLY-OUTPUT"), "{all}");
            text("turn two answer")
        }
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(!r.is_finished(), "{r:?}");
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("again"))
        .await
        .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 6);
    // The audit record keeps the whole child transcript.
    assert!(result_ids(&sd).contains(&"r1".to_string()));
}

/// A sub context does not consume its caller's input queue: a message that
/// arrives while it runs joins the caller's next batch.
#[tokio::test]
async fn input_arriving_during_a_sub_context_goes_to_the_caller() {
    let env = Env::new();
    let mut spec = work_spec("plan then do");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    spec.prompt.behavior = Some("plan".into());
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "do": { "mode": "create_sub_context" } } }),
    );
    let sd = env.create_work(spec).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |req, n| {
        let all = render(&req.messages);
        match n {
            0 => text("<response><next_behavior>do</next_behavior></response>"),
            1 => {
                post_blocking(&qd, &q, msg("extra-info from the user"));
                text("<response><actions><shell><![CDATA[echo do-1]]></shell></actions></response>")
            }
            2 => {
                assert!(!all.contains("extra-info"), "the child must not see it:\n{all}");
                text("<response><report end=\"true\"><![CDATA[did it]]></report></response>")
            }
            3 => {
                assert!(all.contains("did it") && all.contains("extra-info"), "{all}");
                text("<response><report end=\"true\"><![CDATA[final]]></report></response>")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}
