//! L3: work sessions driven end to end with a scripted LLM.

mod common;

use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;
use serde_json::json;

#[tokio::test]
async fn a01_work_session_completes_with_tool_call() {
    let env = Env::new();
    let sd = env.create_work(work_spec("create hello.txt with the text hi")).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => {
            assert!(last_user_text(req).contains("on_init"), "{}", render(&req.messages));
            tool_call(
                "c1",
                "exec",
                json!({ "command": "echo hi > hello.txt && echo $OPENDAN_SESSION_ID" }),
            )
        }
        _ => {
            let r = has_tool_result(req, "c1").expect("tool result visible");
            assert!(r.contains("work-"), "env contract: {r}");
            text("Done: hello.txt contains hi")
        }
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    // Session dir is the default workdir: the file is an output of the session.
    assert_eq!(
        std::fs::read_to_string(sd.path().join("hello.txt")).unwrap().trim(),
        "hi"
    );
    let st = sd.state().unwrap();
    assert_eq!(st.run_state, RunState::Finished);
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!(st.acceptance, Acceptance::Pending);
    assert!(st.live_run.is_none());
    let last = st.last_run.clone().expect("last run kept");
    assert!(sd.runs().exists(&last), "last run directory kept");
    let rec = sd.runs().record(&last).unwrap();
    assert!(rec.inflight.is_empty(), "inflight cleared: {:?}", rec.inflight);
    assert!(rec.executions.is_empty(), "executions confirmed stopped");
    assert!(rec.host_commit_pending.is_none());
    assert_eq!(rec.host.as_ref().unwrap().assembled_by, "libopendan");
    assert!(sd.report().unwrap().contains("hello.txt contains hi"));
    // Worklog: created → turn → message → response → result → response →
    // outcome → turn end. Function call responses are not behavior Steps.
    let wl = read_worklog(&sd);
    let k = kinds(&wl);
    assert_eq!(
        k,
        vec![
            "created",
            "turn_started",
            "user_message",
            "assistant_message",
            "action_result",
            "assistant_message",
            "outcome",
            "turn_ended"
        ],
        "{wl:#?}"
    );
    // 2 Rounds (tool call + final answer), 1 tool iteration, 1 Turn, 1 run.
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.turns, stats.runs), (2, 1, 1));
    assert_eq!(rec.usage.llm_requests, 2);
    let (_, snap) = sd.runs().load_checked(&last).unwrap();
    let snap = snap.unwrap();
    assert!(snap.state.steps.is_empty());
    assert_eq!(
        snap.request.tool_policy.max_tool_iterations - snap.state.tool_iterations_left,
        1
    );
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.open_turn.is_none());
    assert_eq!(wl.last().unwrap().seq, st.worklog.committed_seq);
    // Registry mirrors the committed state.
    let agent = env.agent();
    let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(e.status.rev, st.rev);
    assert_eq!(e.status.run_state, RunState::Finished);
    assert!(e.status.last_runner.is_some());
    // Perception: run digest + task outcome.
    assert_eq!(agent.perception().last_seq(sd.sid()).await.unwrap(), 2);
    // binding + session bin prepared.
    let b = sd.binding_opt().unwrap().unwrap();
    assert_eq!(b.runtime_id, "rt-test-native");
    assert!(sd.runtime_bin_dir().join(".manifest.json").exists());
    // Idle drive of a finished session is a no-op.
    let r2 = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(r2.is_finished());
    assert_eq!(llm.count(), 2);
}

#[tokio::test]
async fn follow_up_input_after_finish_is_rejected_and_logged() {
    let env = Env::new();
    let sd = env.create_work(work_spec("say hi")).await;
    let llm = ScriptedLlm::new(|_, _| text("hi"));
    let deps = env.deps(llm.clone());
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    let agent = env.agent();
    // Posting a normal msg to a finished session is refused up front…
    let err = libopendan::post_input(agent.as_ref(), sd.sid(), &Input::msg("m1", "more?"), APP).await;
    assert!(err.is_err());
    // …and a raw delivery that slipped through is rejected by the consumer.
    let ch = env.channels();
    let q = sd.config().unwrap().channels.kmsg().unwrap().1.to_string();
    libopendan::channel::kmsg::post_to_queue(&ch.client(), &q, &Input::msg("m2", "late"), APP)
        .await
        .unwrap();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(r.is_finished());
    let wl = read_worklog(&sd);
    assert_eq!(wl.last().unwrap().body.kind(), "input_rejected");
    assert_eq!(llm.count(), 1);
    // ack moved past the rejected input.
    let st = sd.state().unwrap();
    assert!(st.source("q").acked_index >= 1);
}

#[tokio::test]
async fn non_driver_is_refused_and_busy_is_reported() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let llm = ScriptedLlm::new(|_, _| text("ok"));
    let mut other = env.deps(llm.clone());
    other.who = "app:app3@alice".into();
    match drive(&sd, &other, StopWhen::Idle).await {
        DriveResult::NotDriver { driver } => assert_eq!(driver, APP),
        r => panic!("{r:?}"),
    }
    // A holder in this process makes another drive Busy.
    let holder = env.deps(llm.clone()).holder();
    let lease = match sd.acquire(holder).unwrap() {
        libopendan::lock::Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    match drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await {
        DriveResult::Busy { holder } => assert!(holder.is_some()),
        r => panic!("{r:?}"),
    }
    drop(lease);
    assert!(drive(&sd, &env.deps(llm), StopWhen::Finished).await.is_finished());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a02_two_runners_share_one_agent_root_concurrently() {
    let env = Env::new();
    let mut dirs = Vec::new();
    for i in 0..4 {
        dirs.push(env.create_work(work_spec(&format!("task {i}"))).await);
    }
    let script = |req: &llm_context::deps::LlmInferenceRequest, _n: usize| {
        if has_tool_result(req, "c1").is_some() {
            text("done")
        } else {
            tool_call("c1", "exec", json!({ "command": "echo $OPENDAN_SESSION_ID > me.txt" }))
        }
    };
    // Two independent runners (separate deps / runner ids), same AgentRoot.
    let r1 = env.deps(ScriptedLlm::new(script));
    let r2 = env.deps(ScriptedLlm::new(script));
    let mut handles = Vec::new();
    for (i, sd) in dirs.iter().cloned().enumerate() {
        let deps = if i % 2 == 0 { r1.clone() } else { r2.clone() };
        handles.push(tokio::spawn(async move { drive(&sd, &deps, StopWhen::Finished).await }));
    }
    for h in handles {
        let r = h.await.unwrap();
        assert!(r.is_finished(), "{r:?}");
    }
    let agent = env.agent();
    for sd in &dirs {
        let me = std::fs::read_to_string(sd.path().join("me.txt")).unwrap();
        assert_eq!(me.trim(), sd.sid());
        let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
        assert_eq!(e.status.run_state, RunState::Finished);
        assert_eq!(e.status.rev, sd.state().unwrap().rev);
        assert_eq!(agent.perception().last_seq(sd.sid()).await.unwrap(), 2);
    }
    // Both runner identities appear as last runners.
    let runners: std::collections::BTreeSet<String> = futures_join(&agent, &dirs).await;
    assert_eq!(runners.len(), 2);
}

async fn futures_join(
    agent: &libopendan::FsAgentStateClient,
    dirs: &[libopendan::SessionDir],
) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for sd in dirs {
        let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
        out.insert(e.status.last_runner.unwrap().runner_id);
    }
    out
}
