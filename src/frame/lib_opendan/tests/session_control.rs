//! Manual control of sessions (notepads/agent-ui-upgrade-todo.md §4.6):
//! every session has a queue for controls (SC0), a stop is for a message or
//! for the session and means what the session's kind says (SC2), an input
//! can be retracted (SC10), a session working when its host restarted is
//! interrupted until resumed by hand (SC11), finished work can be deleted
//! (SC8).

mod common;

use std::sync::Arc;

use common::*;
use libopendan::api::create_sub_session;
use libopendan::api::SubSessionSpec;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionDir, SessionTemplate};
use serde_json::json;

fn work(objective: &str, first: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.prompt.initial_inputs = vec![PostedInput::text(APP, AGENT, first).unwrap()];
    spec
}

fn ui() -> libopendan::api::SessionSpec {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Aalice"));
    spec.owner = Some(USER.into());
    spec.outbound = Some(OutboundBinding {
        to: USER.into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec.prompt.llm_context = json!({ "tools": { "enabled": true, "tools": [{ "groupname": "bash_core" }] } });
    spec
}

fn control(key: &str, command: ControlCommand) -> PostedInput {
    PostedInput::control(APP, key, command)
}

fn end() -> buckyos_api::AiResponse {
    tool_call("end", "report", json!({ "report": "done", "is_end": true }))
}

fn applied(sd: &SessionDir, command: &str) -> Vec<serde_json::Value> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ControlApplied { command: c, detail, .. } if c == command => Some(detail),
            _ => None,
        })
        .collect()
}

/// SC0: a plain work session has a queue that takes controls and no
/// message; what it is created with still opens its first Turn.
#[tokio::test]
async fn a_plain_work_takes_controls_and_no_message() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(work("a bounded task", "do the task")).await;
    let entry = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert!(entry.input_queue.is_some(), "every session has a queue");
    assert_eq!(sd.config().unwrap().session.input_policy, InputPolicy::None);

    let refused = agent.sessions().post_input(sd.sid(), &msg("change of plan")).await.unwrap_err();
    assert!(matches!(&refused, OpenDanError::InvalidArgument(m) if m.contains("takes no messages")), "{refused}");

    // Paused before it starts: nothing is inferred, its first input waits.
    let index = agent
        .sessions()
        .post_input(sd.sid(), &control("pause-1", ControlCommand::Pause { reason: Some("hold".into()) }))
        .await
        .unwrap();
    assert!(index > 0);
    let llm = ScriptedLlm::new(|req, _| {
        assert!(last_user_text(req).contains("do the task"), "{}", last_user_text(req));
        end()
    });
    let deps = env.deps(llm.clone());
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 0);
    let st = sd.state().unwrap();
    assert_eq!(st.paused.as_ref().map(|p| p.source), Some(PauseSource::User));
    assert_eq!(agent.sessions().lookup(sd.sid()).await.unwrap().unwrap().status.paused, Some(PauseSource::User));

    agent
        .sessions()
        .post_input(sd.sid(), &control("resume-1", ControlCommand::Resume { reason: None }))
        .await
        .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 1, "its bootstrap message opened the first Turn");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
}

/// SC2: a stop of the session is refused by a conversation (it only ends
/// when archived); a stop of the message with no Turn open applies to
/// nothing; archiving ends it as stopped.
#[tokio::test]
async fn a_conversation_is_archived_never_stopped() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(ui()).await;
    let llm = ScriptedLlm::new(|_, n| text(&format!("answer {n}")));
    let deps = env.deps(llm.clone());
    agent.sessions().post_input(sd.sid(), &msg("hello")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);

    agent.sessions().post_input(sd.sid(), &control("stop-s", ControlCommand::stop(Some("enough".into())))).await.unwrap();
    agent
        .sessions()
        .post_input(sd.sid(), &control("stop-t", ControlCommand::Stop { reason: None, scope: StopScope::Turn }))
        .await
        .unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert!(!st.is_finished() && !st.stop_requested, "{st:?}");
    let stops = applied(&sd, "stop");
    assert_eq!(stops.len(), 2);
    assert_eq!((stops[0]["scope"].as_str(), stops[0]["applied"].as_bool()), (Some("session"), Some(false)));
    assert!(stops[0]["refused"].as_str().unwrap().contains("new conversation"), "{}", stops[0]);
    assert_eq!((stops[1]["scope"].as_str(), stops[1]["applied"].as_bool()), (Some("turn"), Some(false)));

    // It still answers.
    agent.sessions().post_input(sd.sid(), &msg("hello again")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 2);

    agent
        .sessions()
        .post_input(sd.sid(), &control("archive", ControlCommand::Stop { reason: Some("a new conversation starts".into()), scope: StopScope::Archive }))
        .await
        .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
    let archived = applied(&sd, "stop").pop().unwrap();
    assert_eq!((archived["scope"].as_str(), archived["applied"].as_bool()), (Some("archive"), Some(true)));

    // A work session is not archived.
    let w = env.create_work(work("a task", "go")).await;
    agent
        .sessions()
        .post_input(w.sid(), &control("archive", ControlCommand::Stop { reason: None, scope: StopScope::Archive }))
        .await
        .unwrap();
    assert!(drive(&w, &env.deps(ScriptedLlm::new(|_, _| end())), StopWhen::Finished).await.is_finished());
    assert_eq!(w.state().unwrap().outcome, Some(Outcome::Succeeded));
    assert_eq!(applied(&w, "stop")[0]["applied"].as_bool(), Some(false));
}

/// SC10, an input still queued: retracted, it never enters a Turn.
#[tokio::test]
async fn a_retracted_input_still_queued_never_enters_a_turn() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(ui()).await;
    let llm = ScriptedLlm::new(|req, _| {
        assert!(!render(&req.messages).contains("the wrong address"), "{}", render(&req.messages));
        text("ok")
    });
    let deps = env.deps(llm.clone());
    let wrong = msg("send it to the wrong address");
    agent.sessions().post_input(sd.sid(), &wrong).await.unwrap();
    agent
        .sessions()
        .post_input(sd.sid(), &control("retract-1", ControlCommand::Retract { key: wrong.key.clone(), reason: None }))
        .await
        .unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 0, "nothing was inferred for it");
    assert_eq!(sd.state().unwrap().turn_seq, 0);
    assert_eq!(applied(&sd, "retract")[0]["outcome"], "dropped");
    assert!(read_worklog(&sd).iter().any(|e| matches!(&e.body,
        WorklogBody::InputRejected { reason, input, .. } if reason == "retracted" && input.key == wrong.key)));

    agent.sessions().post_input(sd.sid(), &msg("send it to the right address")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);

    // Its Turn is closed by now: a retraction changes nothing.
    agent
        .sessions()
        .post_input(sd.sid(), &control("retract-2", ControlCommand::Retract { key: wrong.key.clone(), reason: None }))
        .await
        .unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(applied(&sd, "retract")[1]["applied"].as_bool(), Some(false));
}

/// SC2 + SC10, the message in progress: a stop of the message closes its
/// Turn as stopped and the conversation goes on with its context; a
/// retraction does the same and the next Turn knows the input was taken
/// back, with what the Turn had done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_message_in_progress_is_stopped_or_retracted_and_the_conversation_goes_on() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(ui()).await;
    let queue = queue_of(&sd);
    let queue_dir = env.queue_dir.clone();
    let first = msg("look into the first thing");
    let second = msg("look into the second thing");
    let (k1, k2) = (first.key.clone(), second.key.clone());
    let retracted = k2.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        // The user stops the message while its first command runs.
        0 => {
            post_blocking(&queue_dir, &queue, control("stop-1", ControlCommand::Stop { reason: None, scope: StopScope::Turn }));
            tool_call("c1", "shell", json!({ "command": "echo first-step-done" }))
        }
        // The user takes the second message back while it is worked on.
        1 => {
            let seen = render(&req.messages);
            assert!(seen.contains("the first thing") && seen.contains("first-step-done"), "{seen}");
            post_blocking(&queue_dir, &queue, control("retract-1", ControlCommand::Retract { key: retracted.clone(), reason: None }));
            tool_call("c2", "shell", json!({ "command": "echo second-step-done" }))
        }
        _ => {
            let seen = render(&req.messages);
            assert!(seen.contains("the second thing") && seen.contains("second-step-done"), "{seen}");
            assert!(seen.contains("retracted"), "{seen}");
            text("third answer")
        }
    });
    let deps = env.deps(llm.clone());

    agent.sessions().post_input(sd.sid(), &first).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!((llm.count(), st.turn_seq, st.open_turn.is_none(), st.is_finished()), (1, 1, true, false), "{st:?}");
    assert!(!st.turn_stop_requested);
    assert_eq!(applied(&sd, "stop")[0]["applied"].as_bool(), Some(true));

    agent.sessions().post_input(sd.sid(), &second).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!((llm.count(), st.turn_seq, st.open_turn.is_none(), st.is_finished()), (2, 2, true, false), "{st:?}");
    let retract = applied(&sd, "retract").pop().unwrap();
    assert_eq!(retract["outcome"], "stopped");
    assert_eq!(retract["key"].as_str(), Some(k2.as_str()));
    assert_ne!(k1, k2);

    agent.sessions().post_input(sd.sid(), &msg("and now the third thing")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 3);
    assert_eq!(sd.state().unwrap().turns_completed, 1, "only the third Turn completed");
}

fn recovering(env: &Env, llm: Arc<ScriptedLlm>) -> libopendan::runner::RunnerDeps {
    let mut deps = env.deps(llm);
    deps.recovering = Some(libopendan::runner::Recovering::new(libopendan::now_ms()));
    deps
}

/// SC11: after a restart a work session that was working is interrupted,
/// not advanced; a resume reaches the sub sessions interrupted with it and
/// leaves the one its user paused alone. A conversation between two
/// messages and a session waiting for its user are not interrupted.
#[tokio::test]
async fn a_restart_interrupts_what_was_working_until_resumed() {
    let env = Env::new();
    let agent = env.agent();
    let ch = env.channels();
    let llm = ScriptedLlm::new(|_, _| end());

    // A conversation between two messages: left as it is.
    let idle = env.create_work(ui()).await;
    agent.sessions().post_input(idle.sid(), &msg("hello")).await.unwrap();
    let chat = ScriptedLlm::new(|_, _| text("hi"));
    drive(&idle, &env.deps(chat.clone()), StopWhen::Idle).await;
    drive(&idle, &recovering(&env, chat.clone()), StopWhen::Idle).await;
    assert!(idle.state().unwrap().paused.is_none());

    // A work with two sub sessions, one of them paused by its user before.
    let parent = env.create_work(work("the plan", "plan it")).await;
    let sub = |title: &str| SubSessionSpec {
        objective: format!("do {title}"),
        title: Some(title.to_string()),
        msgs: vec!["go".into()],
        key: Some(title.to_string()),
        ..Default::default()
    };
    let a = create_sub_session(agent.as_ref(), APP, parent.sid(), sub("a"), ch.as_ref()).await.unwrap();
    let b = create_sub_session(agent.as_ref(), APP, parent.sid(), sub("b"), ch.as_ref()).await.unwrap();
    agent
        .sessions()
        .post_input(b.sid(), &control("pause-b", ControlCommand::Pause { reason: Some("not now".into()) }))
        .await
        .unwrap();
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(b.state().unwrap().paused.unwrap().source, PauseSource::User);

    for sd in [&parent, &a, &b] {
        let r = drive(sd, &recovering(&env, llm.clone()), StopWhen::Idle).await;
        assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    }
    assert_eq!(llm.count(), 0, "nothing was inferred");
    for sd in [&parent, &a] {
        let st = sd.state().unwrap();
        assert_eq!(st.paused.as_ref().map(|p| p.source), Some(PauseSource::Interrupted), "{}", sd.sid());
        assert_eq!(agent.sessions().lookup(sd.sid()).await.unwrap().unwrap().status.paused, Some(PauseSource::Interrupted));
        assert_eq!(applied(sd, "pause").pop().unwrap()["source"], "interrupted");
    }
    assert_eq!(b.state().unwrap().paused.unwrap().source, PauseSource::User, "the user's pause is kept");

    // Further drives do not move it; only a resume does.
    drive(&a, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(llm.count(), 0);
    agent
        .sessions()
        .post_input(parent.sid(), &control("resume-p", ControlCommand::Resume { reason: None }))
        .await
        .unwrap();
    drive(&parent, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert!(parent.state().unwrap().paused.is_none());
    drive(&a, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(a.state().unwrap().is_finished(), "the resume reached the sub session interrupted with it");
    drive(&b, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(b.state().unwrap().paused.unwrap().source, PauseSource::User);
}

/// SC11: stopping the message a restart interrupted ends the interruption
/// too — the conversation takes its next message without a resume.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_an_interrupted_message_frees_the_conversation() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(ui()).await;
    // A Turn left open in the middle of a command: the "host" dies there.
    let dying = ScriptedLlm::fallible(|_, n| match n {
        0 => Ok(tool_call("c1", "shell", json!({ "command": "echo step" }))),
        _ => Err(llm_context::error::LLMComputeError::Provider {
            failure: llm_context::error::ProviderFailure::Transient,
            message: "the host is going away".into(),
        }),
    });
    agent.sessions().post_input(sd.sid(), &msg("a long request")).await.unwrap();
    drive(&sd, &env.deps(dying), StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert!(st.open_turn.is_some(), "{st:?}");

    let llm = ScriptedLlm::new(|_, _| text("next answer"));
    drive(&sd, &recovering(&env, llm.clone()), StopWhen::Idle).await;
    assert_eq!(sd.state().unwrap().paused.unwrap().source, PauseSource::Interrupted);
    // A message sent meanwhile waits.
    agent.sessions().post_input(sd.sid(), &msg("the next request")).await.unwrap();
    drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(llm.count(), 0);

    agent
        .sessions()
        .post_input(sd.sid(), &control("stop-1", ControlCommand::Stop { reason: None, scope: StopScope::Turn }))
        .await
        .unwrap();
    drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert!(st.paused.is_none() && st.open_turn.is_none(), "{st:?}");
    assert_eq!(llm.count(), 1, "the waiting message was answered");
}

/// SC8: a finished work session is deleted with everything below it; one
/// with unfinished work below, a conversation, are not.
#[tokio::test]
async fn finished_work_is_deleted_with_what_is_below_it() {
    let env = Env::new();
    let agent = env.agent();
    let ch = env.channels();
    let llm = ScriptedLlm::new(|_, _| end());
    let deps = env.deps(llm);
    let parent = env.create_work(work("the work", "go")).await;
    let child = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec { objective: "part".into(), title: Some("part".into()), msgs: vec!["go".into()], key: Some("part".into()), ..Default::default() },
        ch.as_ref(),
    )
    .await
    .unwrap();
    assert!(drive(&parent, &deps, StopWhen::Finished).await.is_finished());
    let refused = agent.sessions().delete(parent.sid()).await.unwrap_err();
    assert!(refused.to_string().contains(child.sid()), "{refused}");
    assert!(drive(&child, &deps, StopWhen::Finished).await.is_finished());

    let conversation = env.create_work(ui()).await;
    assert!(agent.sessions().delete(conversation.sid()).await.is_err());

    let deleted = agent.sessions().delete(parent.sid()).await.unwrap();
    assert_eq!(deleted, vec![parent.sid().to_string(), child.sid().to_string()]);
    for sd in [&parent, &child] {
        assert!(agent.sessions().lookup(sd.sid()).await.unwrap().is_none());
        assert!(!sd.path().exists());
    }
    assert!(agent.sessions().delete(parent.sid()).await.is_err());
}

/// SC7: a conversation's prompt is frozen when it is created. After the
/// agent's `role.md` changes the conversation is behind (what the side
/// panel shows); once its history was compacted it freezes again and takes
/// the agent's current prompt.
#[tokio::test]
async fn a_conversation_takes_the_current_prompt_after_its_history_is_compacted() {
    let env = Env::new();
    let agent = env.agent();
    std::fs::write(env.agent_root.join("role.md"), "You are Jarvis, version one.").unwrap();
    let mut spec = ui();
    spec.freeze = true;
    spec.prompt.history_budget_tokens = Some(2_000);
    let sd = env.create_work(spec).await;
    let frozen = |sd: &SessionDir| sd.config().unwrap().prompt.frozen.unwrap();
    assert!(frozen(&sd).identity.role.contains("version one"));
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("first answer"),
        1 => text("The user sent a long request and got the first answer."),
        2 => {
            assert!(system_text(req).contains("version one"), "the run after the compaction still has the old prompt");
            text("second answer")
        }
        _ => {
            assert!(system_text(req).contains("version two"), "{}", system_text(req));
            text("third answer")
        }
    });
    let deps = env.deps(llm.clone());
    agent.sessions().post_input(sd.sid(), &msg("long request ".repeat(2_000))).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;

    std::fs::write(env.agent_root.join("role.md"), "You are Jarvis, version two.").unwrap();
    let changes = |sd: &SessionDir| {
        let cfg = sd.config().unwrap();
        let agent = agent.clone();
        async move {
            let next = libopendan::state::refrozen(&cfg, agent.behaviors(), agent.agent_root(), APP).await.unwrap();
            libopendan::state::frozen_changes(cfg.prompt.frozen.as_ref().unwrap(), next.prompt.frozen.as_ref().unwrap())
        }
    };
    assert_eq!(changes(&sd).await, ["role.md"]);

    // The second message compacts the history while its context is built.
    agent.sessions().post_input(sd.sid(), &msg("continue")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert!(sd.summary_opt().unwrap().is_some(), "the history was compacted");
    assert_eq!(changes(&sd).await, ["role.md"], "work sessions aside, nothing changes before the next run");

    agent.sessions().post_input(sd.sid(), &msg("and once more")).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 4);
    assert!(frozen(&sd).identity.role.contains("version two"));
    assert!(changes(&sd).await.is_empty());
    let refrozen = applied(&sd, "prompt_refrozen");
    assert_eq!(refrozen.len(), 1);
    assert_eq!(refrozen[0]["changed"], json!(["role.md"]));
    assert_eq!(sd.state().unwrap().turns_completed, 3);
}

fn system_text(req: &llm_context::deps::LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == buckyos_api::AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}
