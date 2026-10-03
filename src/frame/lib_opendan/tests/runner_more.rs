//! L3 / L5 scenarios in one process: stop at an observation boundary,
//! change injection, behavior loop runs, decide / artifact head.

mod common;

use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::state::AgentStateClient;
use serde_json::json;

#[tokio::test]
async fn stop_takes_effect_after_one_do_action() {
    let env = Env::new();
    let sd = env.create_work(work_spec("count to many")).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |_, n| {
        if n == 0 {
            post_blocking(
                &qd,
                &q,
                Input::control(
                    "stop-1",
                    &ControlCommand::Stop {
                        reason: Some("user".into()),
                    },
                ),
            );
            tool_call("c1", "shell", json!({ "command": "echo one >> log" }))
        } else {
            tool_call("c2", "shell", json!({ "command": "echo two >> log" }))
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 1, "no inference after the stop");
    let log = std::fs::read_to_string(sd.path().join("log")).unwrap();
    assert_eq!(log.trim(), "one");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Stopped));
    assert!(!st.stop_requested);
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(
        |e| matches!(&e.body, WorklogBody::ControlApplied { command, .. } if command == "stop")
    ));
    assert!(wl
        .iter()
        .any(|e| matches!(&e.body, WorklogBody::Outcome { kind, .. } if kind == "stopped")));
    assert_eq!(st.source("q").acked_index, 1);
}

#[tokio::test]
async fn stop_before_any_run_finishes_without_inference() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &Input::control("s", &ControlCommand::Stop { reason: None }),
        APP,
    )
    .await
    .unwrap();
    let llm = ScriptedLlm::new(|_, _| text("never"));
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 0);
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
}

#[tokio::test]
async fn change_is_injected_at_the_observation_boundary() {
    let env = Env::new();
    let mut spec = work_spec("watch the camera");
    spec.subscriptions.push(Subscription {
        id: "s2".into(),
        mode: SubscriptionMode::Semi,
        source: SubscriptionSource::ObjectEvent {
            object: "https://cam/01".into(),
            event: "motion".into(),
        },
        watch: vec![],
    });
    let sd = env.create_work(spec).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            // Two progress changes with one key (coalesced) + nothing else.
            for v in ["evt1", "evt2"] {
                post_blocking(
                    &qd,
                    &q,
                    Input::change(
                        "cam01#motion",
                        json!({ "text": format!("motion at door ({v})"), "subscription": "s2", "version": v }),
                    ),
                );
            }
            tool_call("c1", "shell", json!({ "command": "true" }))
        }
        _ => {
            let u = last_user_text(req);
            assert!(u.contains("motion at door (evt2)"), "{u}");
            assert!(!u.contains("evt1"), "coalesced: {u}");
            text("noted")
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    let st = sd.state().unwrap();
    assert_eq!(
        st.source("q").acked_index,
        2,
        "both change deliveries consumed"
    );
    assert_eq!(st.subscription_cursors["s2"]["version"], json!("evt2"));
    let wl = read_worklog(&sd);
    let users: Vec<String> = wl
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::UserMessage { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(users.len(), 2, "{users:?}");
    assert!(users[1].contains("evt2"));
    assert!(wl
        .iter()
        .any(|e| matches!(&e.body, WorklogBody::ChangeDropped { .. })));
    // The observation joined the Turn: one more user message, no new Turn,
    // no input_batch marker.
    let k = kinds(&wl);
    assert_eq!(count(&k, "turn_started"), 1);
    assert_eq!(count(&k, "input_batch"), 0);
    let turns: Vec<u64> = wl
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::UserMessage { turn, .. } => Some(*turn),
            _ => None,
        })
        .collect();
    assert_eq!(turns, vec![1, 1]);
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}

#[tokio::test]
async fn semi_subscribed_session_change_is_pulled_by_rev() {
    let env = Env::new();
    // A finishes first.
    let a = env.create_work(work_spec("task A")).await;
    assert!(drive(
        &a,
        &env.deps(ScriptedLlm::new(|_, _| text("A done"))),
        StopWhen::Finished
    )
    .await
    .is_finished());
    let mut spec = work_spec("task B waits for A");
    spec.subscriptions.push(Subscription {
        id: "sa".into(),
        mode: SubscriptionMode::Semi,
        source: SubscriptionSource::Session {
            session_ref: a.sid().to_string(),
        },
        watch: vec!["run_state".into(), "outcome".into()],
    });
    let b = env.create_work(spec).await;
    let a_sid = a.sid().to_string();
    let llm = ScriptedLlm::new(move |req, _| {
        let u = last_user_text(req);
        assert!(u.contains(&a_sid) && u.contains("finished"), "{u}");
        text("B done")
    });
    assert!(drive(&b, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(
        llm.count(),
        1,
        "semi changes ride along, no extra inference"
    );
    let cur = &b.state().unwrap().subscription_cursors["sa"];
    assert!(cur["rev"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn behavior_loop_session_runs_actions() {
    let env = Env::new();
    let mut spec = work_spec("write out.txt");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        match n {
        0 => text("<response><thinking>go</thinking><actions><shell><![CDATA[echo behavior-77 > out.txt; cat out.txt]]></shell></actions></response>"),
        _ => {
            let u = last_user_text(req);
            assert!(u.contains("behavior-77"), "{u}");
            text("<response><report><![CDATA[out.txt written]]></report></response>")
        }
    }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(
        std::fs::read_to_string(sd.path().join("out.txt"))
            .unwrap()
            .trim(),
        "behavior-77"
    );
    assert!(sd.report().unwrap().contains("out.txt written"));
    let wl = read_worklog(&sd);
    let k = kinds(&wl);
    assert_eq!(
        k,
        vec![
            "created",
            "turn_started",
            "user_message",
            "step",
            "action_result",
            "step",
            "outcome",
            "turn_ended"
        ]
    );
    // Steps carry their run-local identity.
    let steps: Vec<(String, u32, u64)> = wl
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Step {
                run_id,
                step_index,
                turn,
                ..
            } => Some((run_id, step_index, turn)),
            _ => None,
        })
        .collect();
    let run_id = sd.state().unwrap().last_run.unwrap();
    assert_eq!(steps, vec![(run_id.clone(), 0, 1), (run_id, 1, 1)]);
}

#[tokio::test]
async fn decide_accept_and_discard_move_the_artifact_head() {
    let env = Env::new();
    let agent = env.agent();
    let mk = |obj: &str| {
        let mut s = work_spec(obj);
        s.artifact_id = Some("snake-game".into());
        s.workspace = Some(WorkspaceRef::External {
            path: env.root.join("snake").display().to_string(),
        });
        s
    };
    std::fs::create_dir_all(env.root.join("snake")).unwrap();
    let llm = ScriptedLlm::new(|req, _| {
        if has_tool_result(req, "c1").is_some() {
            text("done")
        } else {
            tool_call("c1", "shell", json!({ "command": "echo v >> snake.js" }))
        }
    });
    let decide = |d: &str| {
        Input::control(
            format!("decide-{d}"),
            &ControlCommand::Decide {
                decision: d.into(),
                by: "did:user:alice".into(),
                note: None,
            },
        )
    };
    // A: finish, accept → head = A.
    let a = env.create_work(mk("A")).await;
    assert!(drive(&a, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    let va = format!("v-{}", a.sid());
    assert_eq!(
        agent
            .artifacts()
            .version("snake-game", &va)
            .await
            .unwrap()
            .unwrap()
            .state,
        VersionState::Produced
    );
    libopendan::post_input(agent.as_ref(), a.sid(), &decide("accept"), APP)
        .await
        .unwrap();
    assert!(drive(&a, &env.deps(llm.clone()), StopWhen::Idle)
        .await
        .is_finished());
    assert_eq!(a.state().unwrap().acceptance, Acceptance::Accepted);
    assert_eq!(
        agent
            .artifacts()
            .head("snake-game")
            .await
            .unwrap()
            .unwrap()
            .head,
        Some(va.clone())
    );
    // B inherits A as base; accept → head = B.
    let b = env.create_work(mk("B")).await;
    assert!(drive(&b, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    let vb = format!("v-{}", b.sid());
    assert_eq!(
        agent
            .artifacts()
            .version("snake-game", &vb)
            .await
            .unwrap()
            .unwrap()
            .base,
        Some(va.clone())
    );
    libopendan::post_input(agent.as_ref(), b.sid(), &decide("accept"), APP)
        .await
        .unwrap();
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(
        agent
            .artifacts()
            .head("snake-game")
            .await
            .unwrap()
            .unwrap()
            .head,
        Some(vb.clone())
    );
    // Discarding A later does not override B's head.
    libopendan::post_input(agent.as_ref(), a.sid(), &decide("discard"), APP)
        .await
        .unwrap();
    drive(&a, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(a.state().unwrap().acceptance, Acceptance::Discarded);
    assert_eq!(
        agent
            .artifacts()
            .head("snake-game")
            .await
            .unwrap()
            .unwrap()
            .head,
        Some(vb.clone())
    );
    let rep = &a.state().unwrap().result.unwrap()["discard_report"];
    assert_eq!(rep["workspace"], json!("unsupported"));
    assert!(!rep["unsupported"].as_array().unwrap().is_empty(), "{rep}");
    // Discarding B: its base A is discarded → no valid ancestor → null.
    libopendan::post_input(agent.as_ref(), b.sid(), &decide("discard"), APP)
        .await
        .unwrap();
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(
        agent
            .artifacts()
            .head("snake-game")
            .await
            .unwrap()
            .unwrap()
            .head,
        None
    );
    // Perception carries task_discarded with its source.
    let backlog = agent
        .perception()
        .backlog(&PerceptionCursor::default())
        .await
        .unwrap();
    let mut kinds = Vec::new();
    for item in &backlog.items {
        for r in agent.perception().read(item).await.unwrap() {
            kinds.push(r.kind);
        }
    }
    assert_eq!(
        kinds.iter().filter(|k| *k == "task_discarded").count(),
        2,
        "{kinds:?}"
    );
    // Re-posting the same dedup key is dropped silently…
    libopendan::post_input(agent.as_ref(), b.sid(), &decide("accept"), APP)
        .await
        .unwrap();
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(read_worklog(&b).last().unwrap().body.kind(), "decide");
    // …a new accept after discard is rejected (logged), not applied.
    let again = Input::control(
        "decide-accept-2",
        &ControlCommand::Decide {
            decision: "accept".into(),
            by: "did:user:alice".into(),
            note: None,
        },
    );
    libopendan::post_input(agent.as_ref(), b.sid(), &again, APP)
        .await
        .unwrap();
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(b.state().unwrap().acceptance, Acceptance::Discarded);
    assert_eq!(
        read_worklog(&b).last().unwrap().body.kind(),
        "input_rejected"
    );
}

#[tokio::test]
async fn decide_before_finish_waits_as_pending_decision() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(work_spec("x")).await;
    // Posted before the session runs: shown as pending, applied once finished.
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &Input::control(
            "d",
            &ControlCommand::Decide {
                decision: "accept".into(),
                by: "did:user:alice".into(),
                note: None,
            },
        ),
        APP,
    )
    .await
    .unwrap();
    let r = drive(
        &sd,
        &env.deps(ScriptedLlm::new(|_, _| text("ok"))),
        StopWhen::Finished,
    )
    .await;
    assert!(r.is_finished());
    let st = sd.state().unwrap();
    assert_eq!(st.acceptance, Acceptance::Pending);
    assert!(st.pending_decision.is_some());
    // The next drive applies it.
    drive(
        &sd,
        &env.deps(ScriptedLlm::new(|_, _| text("ok"))),
        StopWhen::Idle,
    )
    .await;
    let st = sd.state().unwrap();
    assert_eq!(st.acceptance, Acceptance::Accepted);
    assert!(st.pending_decision.is_none());
    let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(e.status.acceptance, Acceptance::Accepted);
}

#[tokio::test]
async fn activity_control_and_perception_inputs_are_applied() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |_, n| {
        if n == 0 {
            post_blocking(
                &qd,
                &q,
                Input::control(
                    "act-1",
                    &ControlCommand::Activity {
                        summary: Some("editing collision".into()),
                        touch: vec![Touching {
                            kind: "path".into(),
                            target: "ws:snake/src/collision.js".into(),
                            mode: "write".into(),
                            since_ms: 0,
                        }],
                        clear: false,
                    },
                ),
            );
            post_blocking(
                &qd,
                &q,
                Input::perception(
                    "p-1",
                    json!({ "kind": "observation", "summary": "the door is red", "tags": ["door"] }),
                ),
            );
            tool_call(
                "c1",
                "write_file",
                json!({ "path": "notes.md", "content": "n" }),
            )
        } else {
            text("done")
        }
    });
    let deps = env.deps(llm);
    // Stop after the first round to inspect the running state.
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished());
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(
        |e| matches!(&e.body, WorklogBody::ControlApplied { command, .. } if command == "activity")
    ));
    let agent = env.agent();
    let backlog = agent
        .perception()
        .backlog(&PerceptionCursor::default())
        .await
        .unwrap();
    let recs = agent.perception().read(&backlog.items[0]).await.unwrap();
    assert!(recs
        .iter()
        .any(|r| r.kind == "observation" && r.summary == "the door is red"));
    let seqs: Vec<u64> = recs.iter().map(|r| r.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "strictly increasing perception seq");
}

fn behavior_spec(obj: &str, modes: serde_json::Value) -> libopendan::api::SessionSpec {
    let mut spec = work_spec(obj);
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    spec.prompt.behavior = Some("plan".into());
    spec.extensions
        .insert("opendan".into(), json!({ "process_modes": modes }));
    spec
}

fn step_texts(sd: &libopendan::SessionDir) -> Vec<String> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Step { assistant, .. } => Some(assistant),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn fork_child_inherits_steps_and_returns_to_the_parent_run() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "research then answer",
            json!({ "research": "fork" }),
        ))
        .await;
    let llm = ScriptedLlm::new(|req, n| {
        let all = render(&req.messages);
        match n {
            0 => text("<response><actions><shell><![CDATA[echo p1-output]]></shell></actions></response>"),
            1 => text("<response><thinking>need research</thinking><next_behavior>research</next_behavior></response>"),
            2 => {
                assert!(all.contains("p1-output"), "child inherits parent steps:\n{all}");
                assert!(all.contains("behavior_switch to=\"research\""), "{all}");
                text("<response><actions><shell><![CDATA[echo r1-output]]></shell></actions></response>")
            }
            3 => text("<response><report><![CDATA[research result X]]></report></response>"),
            4 => {
                assert!(all.contains("research result X"), "parent sees the child result:\n{all}");
                assert!(!all.contains("r1-output"), "child steps stay in the child:\n{all}");
                text("<response><report><![CDATA[final answer]]></report></response>")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 5);
    assert!(sd.report().unwrap().contains("final answer"));
    let st = sd.state().unwrap();
    assert!(st.process_stack.is_empty());
    // Worklog keeps time order; inherited steps are written once.
    let k = kinds(&read_worklog(&sd));
    let outcomes: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Outcome { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes, vec!["suspended", "process_done", "done"], "{k:?}");
    let steps = step_texts(&sd);
    assert_eq!(
        steps.iter().filter(|s| s.contains("p1-output")).count(),
        1,
        "{steps:?}"
    );
    assert_eq!(
        steps.iter().filter(|s| s.contains("r1-output")).count(),
        1,
        "{steps:?}"
    );
    // One logical Turn: the fork call and the return are hand-over batches.
    assert_eq!(count(&k, "turn_started"), 1, "{:#?}", read_worklog(&sd));
    assert_eq!(count(&k, "input_batch"), 2, "{:#?}", read_worklog(&sd));
    assert_eq!(count(&k, "turn_ended"), 1);
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    // 5 Rounds over two runs (parent + child), one Turn.
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.runs, stats.turns), (5, 2, 1));
    // Step identity is (run_id, step_index): the child's steps continue the
    // parent's numbering and are not mixed into the parent's.
    let mut ids: Vec<(String, u32)> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Step {
                run_id, step_index, ..
            } => Some((run_id, step_index)),
            _ => None,
        })
        .collect();
    let all = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all, "each Step written once");
    // Parent run is the last run; the child run was replaced and removed.
    assert_eq!(sd.runs().list().unwrap(), vec![st.last_run.unwrap()]);
    // call ids stay unique across parent and child.
    let mut ids: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

fn count(k: &[&str], what: &str) -> usize {
    k.iter().filter(|x| **x == what).count()
}

#[tokio::test]
async fn independent_processes_keep_their_own_runs() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "alternate",
            json!({ "writer": "independent", "plan": "independent" }),
        ))
        .await;
    let llm = ScriptedLlm::new(|req, n| {
        let all = render(&req.messages);
        match n {
            0 => {
                text("<response><actions><shell><![CDATA[echo plan-1]]></shell></actions></response>")
            }
            1 => text("<response><next_behavior>writer</next_behavior></response>"),
            2 => {
                // Session history (worklog) is shared; live steps are not.
                assert!(
                    !all.contains("step_record behavior=\"plan\""),
                    "no step inheritance\n{all}"
                );
                text("<response><actions><shell><![CDATA[echo writer-1]]></shell></actions></response>")
            }
            3 => text("<response><next_behavior>plan</next_behavior></response>"),
            4 => {
                // Back in plan's own run: its earlier steps are there.
                assert!(all.contains("plan-1") && !all.contains("writer-1"), "{all}");
                text("<response><report><![CDATA[done alternating]]></report></response>")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 5);
    let steps = step_texts(&sd);
    assert_eq!(steps.iter().filter(|s| s.contains("plan-1")).count(), 1);
    assert_eq!(steps.iter().filter(|s| s.contains("writer-1")).count(), 1);
    // Switching contexts is not completing a Turn.
    let k = kinds(&read_worklog(&sd));
    assert_eq!(count(&k, "turn_started"), 1);
    assert_eq!(count(&k, "input_batch"), 2);
    assert_eq!(count(&k, "turn_ended"), 1);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}

#[tokio::test]
async fn normal_switch_continues_the_same_run() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec("two phases", json!({})))
        .await;
    let llm = ScriptedLlm::new(|req, n| {
        let all = render(&req.messages);
        match n {
            0 => text(
                "<response><actions><shell><![CDATA[echo phase-1]]></shell></actions></response>",
            ),
            1 => text("<response><next_behavior>do</next_behavior></response>"),
            2 => {
                assert!(
                    all.contains("phase-1") && all.contains("behavior_switch to=\"do\""),
                    "{all}"
                );
                text("<response><report><![CDATA[both phases done]]></report></response>")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.current_behavior.as_deref(), Some("do"));
    // One run for both behaviors.
    let runs: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::TurnStarted { run_id, .. } | WorklogBody::InputBatch { run_id, .. } => {
                Some(run_id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0], runs[1]);
    // Three Rounds, one run, one logical Input → result.
    let k = kinds(&read_worklog(&sd));
    assert_eq!(count(&k, "turn_started"), 1);
    assert_eq!(count(&k, "input_batch"), 1);
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.runs, stats.turns), (3, 1, 1));
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}

#[tokio::test]
async fn tmux_runtime_runs_exec_in_the_session_pane() {
    if !libopendan::runtime::TmuxRuntime::available() {
        eprintln!("tmux not available; skipped");
        return;
    }
    let env = Env::new();
    let sd = env.create_work(work_spec("tmux exec")).await;
    let llm = ScriptedLlm::new(|req, _| match has_tool_result(req, "c1") {
        Some(r) => {
            assert!(r.contains("from-tmux") && r.contains("work-"), "{r}");
            text("tmux ok")
        }
        None => tool_call(
            "c1",
            "shell",
            json!({ "command": "echo from-tmux; echo $OPENDAN_SESSION_ID; echo x > t.txt" }),
        ),
    });
    let mut deps = env.deps(llm.clone());
    let rt = std::sync::Arc::new(libopendan::runtime::TmuxRuntime::from_config(
        agent_tool::runtime::RuntimeConfig {
            kind: Some("tmux".into()),
            id: Some("rt-test-tmux".into()),
            tmux: Some(agent_tool::runtime::TmuxConfig {
                session: Some(libopendan::runtime::tmux::tmux_session_name(sd.sid())),
                ..Default::default()
            }),
            ..Default::default()
        },
    ));
    deps.runtime = rt;
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(sd.path().join("t.txt").exists());
    assert_eq!(sd.binding_opt().unwrap().unwrap().kind, "tmux");
    let name = libopendan::runtime::tmux::tmux_session_name(sd.sid());
    let has = std::process::Command::new("tmux")
        .args(["has-session", "-t", &name])
        .status()
        .unwrap();
    assert!(has.success(), "tmux session kept for audit");
    let _ = std::process::Command::new("tmux")
        .args(["kill-session", "-t", &name])
        .status();
}

#[tokio::test]
async fn semi_change_alone_does_not_make_an_input_batch() {
    let env = Env::new();
    let mut spec = work_spec("wait for messages");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    spec.subscriptions.push(Subscription {
        id: "s2".into(),
        mode: SubscriptionMode::Semi,
        source: SubscriptionSource::ObjectEvent {
            object: "o".into(),
            event: "e".into(),
        },
        watch: vec![],
    });
    let sd = env.create_work(spec).await;
    // The first batch waits for the user without a reply: the Turn stays
    // open.
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>"),
        _ => {
            let u = last_user_text(req);
            assert!(u.contains("door opened") && u.contains("hello"), "{u}");
            text("<response><report><![CDATA[ok]]></report></response>")
        }
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(
            r,
            libopendan::runner::DriveResult::Idle {
                run_state: RunState::Waiting,
                ..
            }
        ),
        "{r:?}"
    );
    // A semi change alone: no inference.
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &Input::change("door", json!({"text": "door opened", "subscription": "s2"})),
        APP,
    )
    .await
    .unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1, "semi change must not trigger inference");
    assert_eq!(
        sd.state().unwrap().open_turn.as_ref().map(|t| t.index),
        Some(1)
    );
    // A message makes a batch; the change rides along; it joins the Turn
    // that is still waiting for its input.
    libopendan::post_input(agent.as_ref(), sd.sid(), &Input::msg("m1", "hello"), APP)
        .await
        .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 2);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.open_turn.is_none());
    let k = kinds(&read_worklog(&sd));
    assert_eq!(count(&k, "turn_started"), 1);
    assert_eq!(count(&k, "input_batch"), 1);
}

#[tokio::test]
async fn normal_switch_across_drives_runs_the_next_behavior() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec("two phases", json!({})))
        .await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("<response><next_behavior>do</next_behavior></response>"),
        _ => {
            assert!(render(&req.messages).contains("behavior_switch to=\"do\""));
            text("<response><report><![CDATA[both phases done]]></report></response>")
        }
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::MaxOutcomes { n: 1 }).await;
    assert!(
        matches!(r, libopendan::runner::DriveResult::OutcomesHandled { .. }),
        "{r:?}"
    );
    let st = sd.state().unwrap();
    let rec = sd
        .runs()
        .record(&st.live_run.clone().unwrap().run_id)
        .unwrap();
    assert!(
        !rec.status.is_terminal(),
        "switched run must not look finished: {:?}",
        rec.status
    );
    // The hand-over did not complete the Turn.
    assert_eq!(st.open_turn.as_ref().map(|t| t.index), Some(1));
    assert_eq!(st.turns_completed, 0);
    // n = 0 only recovers: nothing runs.
    let r = drive(&sd, &deps, StopWhen::MaxOutcomes { n: 0 }).await;
    assert!(
        matches!(r, libopendan::runner::DriveResult::OutcomesHandled { .. }),
        "{r:?}"
    );
    assert_eq!(llm.count(), 1);
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 2, "behavior `do` ran");
    assert!(sd.report().unwrap().contains("both phases done"));
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}

#[tokio::test]
async fn max_turns_counts_completed_turns_not_hand_overs() {
    let env = Env::new();
    let mut spec = behavior_spec("two requests", json!({}));
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| {
        let all = render(&req.messages);
        if all.contains("second request") {
            text("<response><report><![CDATA[answer two]]></report></response>")
        } else if all.contains("behavior_switch to=\"do\"") {
            text("<response><report><![CDATA[answer one]]></report></response>")
        } else {
            text("<response><next_behavior>do</next_behavior></response>")
        }
    });
    let deps = env.deps(llm.clone());
    // Turn 1: plan hands over to do (same Turn), do answers.
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(
            r,
            libopendan::runner::DriveResult::Idle {
                run_state: RunState::Waiting,
                ..
            }
        ),
        "{r:?}"
    );
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.open_turn.is_none());
    // Turn 2 finishes the session.
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &Input::msg("m2", "second request"),
        APP,
    )
    .await
    .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 3);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (2, 2));
    let k = kinds(&read_worklog(&sd));
    assert_eq!(count(&k, "turn_started"), 2);
    assert_eq!(count(&k, "input_batch"), 1, "the hand-over joined Turn 1");
    assert_eq!(count(&k, "turn_ended"), 2);
    assert!(sd.report().unwrap().contains("- turns: 2"));
    assert_eq!(sd.statistics().unwrap().turns, 2);
}

#[tokio::test]
async fn stop_right_after_a_fork_return_closes_the_parent_run() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "research then answer",
            json!({ "research": "fork" }),
        ))
        .await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |req, _| {
        let all = render(&req.messages);
        if all.contains("research result X") {
            text("<response><report><![CDATA[final answer]]></report></response>")
        } else if all.contains("behavior_switch to=\"research\"") {
            post_blocking(
                &qd,
                &q,
                Input::control("stop-1", &ControlCommand::Stop { reason: None }),
            );
            text("<response><report><![CDATA[research result X]]></report></response>")
        } else {
            text("<response><next_behavior>research</next_behavior></response>")
        }
    });
    let r = drive(&sd, &env.deps(llm), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Stopped));
    assert!(st.live_run.is_none(), "finished session keeps no live run");
    assert!(st.process_stack.is_empty());
    // Referenced runs only.
    assert_eq!(sd.runs().list().unwrap(), vec![st.last_run.unwrap()]);
}

#[tokio::test]
async fn call_ids_stay_unique_after_a_fork_return() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec("fork and act", json!({ "research": "fork" })))
        .await;
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => text("<response><actions><shell><![CDATA[echo p1]]></shell></actions></response>"),
        1 => text("<response><next_behavior>research</next_behavior></response>"),
        2 => text("<response><actions><shell><![CDATA[echo r1]]></shell></actions></response>"),
        3 => text("<response><report><![CDATA[research result X]]></report></response>"),
        4 => text(
            "<response><actions><shell><![CDATA[echo after-return]]></shell></actions></response>",
        ),
        _ => text("<response><report><![CDATA[final]]></report></response>"),
    });
    assert!(drive(&sd, &env.deps(llm), StopWhen::Finished)
        .await
        .is_finished());
    let ids: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    let mut u = ids.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), 3, "call ids reused: {ids:?}");
}
