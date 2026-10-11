//! Asking the user from a sub context (`agent-session ask-user`) and the
//! named functions of behaviors (`[tool]`): intent analysis TODO V05–V10,
//! V12, V14, V15, V22.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponse};
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{
    drive, DriveResult, OutboundSink, RunnerDeps, SendResult, StopWhen, TurnTaskEnd,
    TurnTaskOpen, TurnTaskSink, TurnTaskStatus,
};
use libopendan::SessionTemplate;
use llm_context::deps::LlmInferenceRequest;
use ndn_lib::{MachineContent, MsgObject, MsgRelType, MsgRelation, ObjId};
use serde_json::{json, Value};

const BOB: &str = "did:bns:bob";

#[derive(Default)]
struct Tasks {
    updates: Mutex<Vec<TurnTaskStatus>>,
    closed: Mutex<Vec<TurnTaskEnd>>,
    opened: Mutex<u32>,
}

#[async_trait]
impl TurnTaskSink for Tasks {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        *self.opened.lock().unwrap() += 1;
        Ok(format!("t-{}-{}", turn.session_id, turn.turn))
    }

    async fn update(&self, _task_id: &str, status: &TurnTaskStatus) -> Result<(), String> {
        self.updates.lock().unwrap().push(status.clone());
        Ok(())
    }

    async fn close(&self, _task_id: &str, end: &TurnTaskEnd) -> Result<(), String> {
        self.closed.lock().unwrap().push(end.clone());
        Ok(())
    }
}

#[derive(Default)]
struct Sink {
    editable: bool,
    sent: Mutex<Vec<OutboundRecord>>,
}

impl Sink {
    fn sent(&self) -> Vec<OutboundRecord> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl OutboundSink for Sink {
    async fn placeholder(&self, _cfg: &SessionConfig, mut base: MsgObject, _turn: u64) -> Option<MsgObject> {
        if !self.editable {
            return None;
        }
        base.content.content = "working on it".into();
        Some(base)
    }

    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        self.sent.lock().unwrap().push(record.clone());
        SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: vec!["d-1".into()],
        }
    }
}

/// A UI session whose entry context offers `analyze` (a fork) as the
/// function `try_analyze(request)`.
fn ui_spec(analyze: Value) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [ { "groupname": "bash" }, { "name": "behavior:analyze" }, { "name": "report" } ] }
    });
    spec.extensions
        .insert("opendan".into(), json!({ "behaviors": { "analyze": analyze } }));
    spec.route_key = Some(format!("{AGENT}/dm%3A{BOB}"));
    spec.outbound = Some(OutboundBinding {
        to: BOB.into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec
}

fn analyze_entry() -> Value {
    json!({
        "mode": "fork",
        "tool": {
            "name": "try_analyze",
            "description": "Analyze what the user wants and create what it needs.",
            "parameters": { "type": "object", "required": ["request"],
                            "properties": { "request": { "type": "string" } } },
            "task": "Request: {{args.request}}"
        }
    })
}

fn deps_with(env: &Env, llm: Arc<ScriptedLlm>, sink: &Arc<Sink>, tasks: &Arc<Tasks>) -> RunnerDeps {
    let mut deps = env
        .deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.outbound = Some(sink.clone());
    deps.turn_tasks = Some(tasks.clone());
    deps.options.placeholder_delay = Duration::from_secs(3600);
    deps
}

fn from_bob(t: &str, relates_to: Option<ObjId>, machine: Option<Value>) -> PostedInput {
    let mut m = text_msg(&parse_did(BOB).unwrap(), &parse_did(AGENT).unwrap(), t);
    m.relates_to = relates_to.map(|target| MsgRelation::new(MsgRelType::Thread, target));
    m.content.machine = machine.map(|v| serde_json::from_value::<MachineContent>(v).unwrap());
    PostedInput::msg(APP, m, MsgDelivery::default()).unwrap()
}

fn tool_specs(req: &LlmInferenceRequest) -> Vec<String> {
    req.tool_specs.iter().map(|t| t.name.clone()).collect()
}

fn ask(call_id: &str, args: &str) -> AiResponse {
    tool_call(
        call_id,
        "shell",
        json!({ "command": format!("agent-session ask-user {args}") }),
    )
}

fn answer_of(req: &LlmInferenceRequest, call_id: &str) -> UserInputAnswer {
    let r = has_tool_result(req, call_id).unwrap_or_else(|| panic!("no result for {call_id}"));
    serde_json::from_str(&r).unwrap_or_else(|e| panic!("{e}: {r}"))
}

fn by_purpose(sd: &libopendan::SessionDir, p: OutboxPurpose) -> Vec<OutboxEntry> {
    sd.state()
        .unwrap()
        .outbox
        .into_iter()
        .filter(|e| e.purpose == p)
        .collect()
}

/// V05 / V06 / V08 / V15 / V22: the UI's model sees the named function only;
/// the fork branch asks with `ask-user`; the placeholder goes out at once,
/// the question as a `thread` message on it; the Turn stays open waiting
/// for input; the user's next message is the call's result and the branch's
/// end is the function's result; the placeholder becomes the reply.
#[tokio::test]
async fn a_fork_asks_the_user_and_the_answer_is_the_call_result() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(analyze_entry())).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("tidy up the docs", None, None));
    let ui_prefix = Arc::new(Mutex::new(Vec::<AiMessage>::new()));
    let up = ui_prefix.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            let tools = tool_specs(req);
            assert!(tools.contains(&"try_analyze".to_string()), "{tools:?}");
            assert!(!tools.contains(&"call_behavior".to_string()), "{tools:?}");
            *up.lock().unwrap() = req.messages.clone();
            tool_call("f1", "try_analyze", json!({ "request": "tidy up the project docs" }))
        }
        1 => {
            // The branch: the UI's request so far is its prefix, then one
            // hand-over message.
            let prefix = up.lock().unwrap().clone();
            assert_eq!(&req.messages[..prefix.len()], &prefix[..], "byte-equal prefix");
            assert_eq!(req.messages.len(), prefix.len() + 1);
            assert_eq!(tool_specs(req), tool_specs(req), "same tools");
            let last = last_user_text(req);
            assert!(last.contains("<sub_task mode=\"fork\">Request: tidy up the project docs"), "{last}");
            assert!(last.contains("agent-session ask-user"), "{last}");
            ask("q1", "'docs/ only, or the whole repository?'")
        }
        2 => {
            let a = answer_of(req, "q1");
            assert_eq!(a.text, "docs/ only");
            assert!(a.in_reply_to_question, "{a:?}");
            assert_eq!(a.option, None);
            assert_eq!(a.from, BOB);
            tool_call(
                "r1",
                "report",
                json!({
                    "report": "created work w-1 for docs/",
                    "result": { "outcome": "work_created", "clarifications": [ { "question": "docs/ or all?", "answer": "docs/ only" } ] },
                    "is_end": true
                }),
            )
        }
        3 => {
            let r = has_tool_result(req, "f1").unwrap();
            let v: Value = serde_json::from_str(&r).unwrap_or_else(|e| panic!("{e}: {r}"));
            assert_eq!(v["status"], "ok", "{v}");
            assert_eq!(v["behavior"], "analyze");
            assert_eq!(v["submission"]["report"], "created work w-1 for docs/");
            assert_eq!(v["submission"]["result"]["outcome"], "work_created");
            assert_eq!(v["submission"]["result"]["clarifications"][0]["answer"], "docs/ only");
            assert!(has_tool_result(req, "q1").is_none(), "the branch stays in the branch");
            text("Started: tidying docs/.")
        }
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let r = drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(llm.count(), 2);
    let st = sd.state().unwrap();
    assert_eq!(st.run_state, RunState::Waiting);
    let wf = st.waiting_for.clone().unwrap();
    assert_eq!((wf.kind, wf.refs.clone()), (WaitingKind::Input, vec!["userinput:q1".to_string()]));
    assert!(st.open_turn.is_some());
    assert_eq!(st.turns_completed, 0);
    // P then Q, Q a thread on P carrying agent_ask.
    let sent = sink.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].key.ends_with(":placeholder"));
    let p_id = msg_key(&sent[0].msg);
    let qmsg = &sent[1].msg;
    assert_eq!(qmsg.content.content, "docs/ only, or the whole repository?");
    let rel = qmsg.relates_to.as_ref().unwrap();
    assert_eq!((rel.rel.clone(), rel.target.to_string()), (MsgRelType::Thread, p_id.clone()));
    let meta = &qmsg.meta[AGENT_ASK_META];
    assert_eq!(meta["anchor"], p_id.as_str());
    assert_eq!(meta["call_id"], "q1");
    assert_eq!(meta["input"]["kind"], "text");
    assert_eq!(by_purpose(&sd, OutboxPurpose::Question).len(), 1);
    // The Turn's task waits for the answer.
    let last = tasks.updates.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.message, "waiting for the user's answer");

    // Driving again without an answer changes nothing and sends nothing.
    let r = drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!((llm.count(), sink.sent().len()), (2, 2));

    // The answer, a reply on P; a second message stays queued for later.
    let p = ObjId::new(&p_id).unwrap();
    post_blocking(&env.queue_dir, &q, from_bob("docs/ only", Some(p), None));
    let r = drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(llm.count(), 4);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.open_turn.is_none() && st.process_stack.is_empty());
    let sent = sink.sent();
    assert_eq!(sent.len(), 3, "{sent:?}");
    let reply = &sent[2].msg;
    assert_eq!(reply.content.content, "Started: tidying docs/.");
    let rel = reply.relates_to.as_ref().unwrap();
    assert_eq!((rel.rel.clone(), rel.target.to_string()), (MsgRelType::Edit, p_id));
    assert_eq!(*tasks.opened.lock().unwrap(), 1, "one Turn, one task");
    assert_eq!(tasks.closed.lock().unwrap().len(), 1);
    // The answer was consumed as an input of the open Turn.
    let src = sd.config().unwrap().channels.kmsg().unwrap().0.to_string();
    assert!(st.source(&src).is_consumed(2));
    // The UI history rebuilt later has the call and its result only.
    let k = kinds(&read_worklog(&sd));
    assert_eq!(k.iter().filter(|x| **x == "turn_started").count(), 1);
}

/// V07 / V06: a choice question; an answer picked from the options carries
/// its id; an answer that is not a reply still answers, marked as such.
#[tokio::test]
async fn a_choice_answer_carries_its_option() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(analyze_entry())).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("set up the project", None, None));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "set up the project" })),
        1 => ask("q1", "'Where should it live?' --option 'a=~/projects/demo' --option b='a new workspace' --allow-text"),
        2 => {
            let a = answer_of(req, "q1");
            assert_eq!((a.option.as_deref(), a.text.as_str()), (Some("a"), "~/projects/demo"));
            assert!(a.in_reply_to_question);
            ask("q2", "'Shall I start right away?'")
        }
        3 => {
            let a = answer_of(req, "q2");
            assert_eq!((a.text.as_str(), a.option.as_deref()), ("yes", None));
            assert!(!a.in_reply_to_question, "not a reply: {a:?}");
            text("created")
        }
        4 => text("done"),
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    let sent = sink.sent();
    let qmsg = &sent[1].msg;
    let input: UserInputSpec =
        serde_json::from_value(qmsg.meta[AGENT_ASK_META]["input"].clone()).unwrap();
    assert_eq!(input.kind, UserInputKind::Choice);
    assert_eq!(input.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert!(input.allow_text);
    assert!(qmsg.content.content.contains("1. ~/projects/demo\n2. a new workspace"), "{}", qmsg.content.content);
    let anchor = qmsg.relates_to.as_ref().unwrap().target.clone();
    post_blocking(
        &env.queue_dir,
        &q,
        from_bob(
            "~/projects/demo",
            Some(anchor),
            Some(json!({ "intent": AGENT_ASK_ANSWER_INTENT, "data": { "call_id": "q1", "option": "a" } })),
        ),
    );
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 3);
    let questions = by_purpose(&sd, OutboxPurpose::Question);
    assert_eq!(questions.len(), 2, "one question per call");
    post_blocking(&env.queue_dir, &q, from_bob("yes", None, None));
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 5);
    assert_eq!(sd.state().unwrap().turns_completed, 1);
}

/// V05 (no placeholder): a channel that cannot edit gets the question as a
/// message of its own, `agent_ask.anchor` empty.
#[tokio::test]
async fn without_a_placeholder_the_question_stands_alone() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(analyze_entry())).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("do it", None, None));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "do it" })),
        1 => ask("q1", "'Which one?'"),
        2 => {
            assert!(answer_of(req, "q1").in_reply_to_question);
            text("ok")
        }
        3 => text("done"),
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    let sent = sink.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let qmsg = &sent[0].msg;
    assert!(qmsg.relates_to.is_none());
    assert_eq!(qmsg.meta[AGENT_ASK_META]["anchor"], "");
    // A reply to the question itself answers it.
    post_blocking(&env.queue_dir, &q, from_bob("the first", Some(ObjId::new(&msg_key(qmsg)).unwrap()), None));
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 4);
    assert_eq!(sink.sent().len(), 2);
}

/// V12: a stop while the user is being asked ends the Turn; the question
/// call gets a result, the placeholder its closing text.
#[tokio::test]
async fn a_stop_while_asking_ends_the_turn() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(analyze_entry())).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("do it", None, None));
    let llm = ScriptedLlm::new(move |_req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "do it" })),
        1 => ask("q1", "'Which one?'"),
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    post_blocking(
        &env.queue_dir,
        &q,
        PostedInput::control(APP, "stop-1", ControlCommand::stop(Some("not now".into()))),
    );
    let r = drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert!(r.is_finished() || matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert!(st.open_turn.is_none());
    assert!(st.process_stack.is_empty() && st.waiting_for.as_ref().is_none_or(|w| w.kind != WaitingKind::Input || w.refs.is_empty()));
    assert_eq!(llm.count(), 2);
    let edits = by_purpose(&sd, OutboxPurpose::FinalEdit);
    assert_eq!(edits.len(), 1, "the placeholder is ended");
}

/// V09: a session nobody reads (a Work) cannot ask: the call is an error
/// result and the run goes on.
#[tokio::test]
async fn a_work_session_cannot_ask() {
    let env = Env::new();
    let mut spec = SessionTemplate::load("work", None).unwrap().spec("decide alone");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    let sd = env.create_work(spec).await;
    assert!(sd.config().unwrap().channels.kmsg().is_none());
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => ask("q1", "'Which one?'"),
        1 => {
            let r = has_tool_result(req, "q1").unwrap();
            assert!(r.contains("cannot ask the user"), "{r}");
            text("decided by myself")
        }
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    let r = drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert!(sd.report().unwrap().contains("decided by myself"));
}

/// V14: a sub context of a sub context asks; the answer lands in the
/// innermost run.
#[tokio::test]
async fn a_nested_branch_asks() {
    let env = Env::new();
    let mut spec = ui_spec(analyze_entry());
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": {
            "analyze": analyze_entry(),
            "deep": { "mode": "fork" }
        } }),
    );
    spec.prompt.llm_context["tools"]["tools"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "call_behavior" }));
    let sd = env.create_work(spec).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("go", None, None));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "go" })),
        1 => tool_call("d1", "call_behavior", json!({ "behavior": "deep", "task": "dig" })),
        2 => ask("q1", "'Deep question?'"),
        3 => {
            assert_eq!(answer_of(req, "q1").text, "deep answer");
            text("deep done")
        }
        4 => {
            assert!(has_tool_result(req, "d1").unwrap().contains("deep done"));
            text("analysis done")
        }
        5 => text("all done"),
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(sd.state().unwrap().process_stack.len(), 2);
    post_blocking(&env.queue_dir, &q, from_bob("deep answer", None, None));
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 6);
    assert_eq!(sd.state().unwrap().turns_completed, 1);
}

/// V22: only a whitelisted `[tool]` target becomes a function; duplicate
/// function names and offers of targets without `[tool]` are refused.
#[tokio::test]
async fn behavior_functions_come_from_tool_declarations() {
    let env = Env::new();
    // `plain` has no [tool]: only `call_behavior` could reach it.
    let mut spec = ui_spec(analyze_entry());
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "analyze": analyze_entry(), "plain": { "mode": "fork" } } }),
    );
    let sd = env.create_work(spec).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), from_bob("hi", None, None));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            let tools = tool_specs(req);
            assert!(tools.contains(&"try_analyze".to_string()));
            assert!(!tools.iter().any(|t| t == "plain" || t == "call_behavior"), "{tools:?}");
            let spec = req.tool_specs.iter().find(|t| t.name == "try_analyze").unwrap();
            assert_eq!(spec.description, "Analyze what the user wants and create what it needs.");
            text("hello")
        }
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);

    let mut cfg = sd.config().unwrap();
    cfg.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": {
            "analyze": analyze_entry(),
            "other": { "mode": "fork", "tool": { "name": "try_analyze", "description": "x" } }
        } }),
    );
    assert!(cfg.behaviors().unwrap_err().contains("both declare the function `try_analyze`"));
    cfg.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": {
            "chat": { "mode": "switch_context",
                      "llm_context": { "tools": { "tools": [ { "name": "behavior:plain" } ] } } },
            "plain": { "mode": "fork" }
        } }),
    );
    assert!(cfg.behaviors().unwrap_err().contains("behavior:plain"));
    cfg.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "x": { "mode": "switch_context", "tool": { "description": "x" } } } }),
    );
    assert!(cfg.behaviors().unwrap_err().contains("sub context"));
    cfg.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "x": { "mode": "fork", "tool": { "description": "x", "task": "{{session.id}}" } } } }),
    );
    assert!(cfg.behaviors().unwrap_err().contains("only args.<name>"));
}

#[test]
fn task_templates_take_arguments_only() {
    let t = BehaviorToolConfig {
        name: Some("f".into()),
        description: "d".into(),
        parameters: Some(json!({ "type": "object" })),
        task: Some("Do {{ args.what }} for {{args.who}}.".into()),
    };
    t.validate("b").unwrap();
    assert_eq!(t.render_task(&json!({ "what": "X", "who": 7 })), "Do X for 7.");
    let plain = BehaviorToolConfig {
        description: "d".into(),
        ..Default::default()
    };
    assert_eq!(plain.function_name("research"), "research");
    assert_eq!(plain.render_task(&json!({ "task": "find X" })), "find X");
    assert!(BehaviorToolConfig { name: Some("a-b".into()), ..plain.clone() }.validate("b").is_err());
}

/// The fork target's `[budget]` limits the branch: entry tables keep no
/// budget keys for a fork, and freezing a fork with a budget succeeds.
#[test]
fn a_fork_may_declare_a_budget() {
    let b: BehaviorConfig = toml::from_str(
        r#"
        [entry]
        mode = "fork"
        [prompt]
        on_context_switch = "go"
        [budget]
        max_tool_iterations = 40
        max_consecutive_errors = 4
        [tool]
        name = "try_x"
        description = "x"
        "#,
    )
    .unwrap();
    let e = b.to_entry("x", &json!({ "model": "m" }), false).unwrap();
    assert!(e.llm_context.is_null(), "{}", e.llm_context);
    assert_eq!(e.tool.unwrap().function_name("x"), "try_x");
    let with_model: BehaviorConfig = toml::from_str(
        "[entry]\nmode = \"fork\"\n[model]\npreferred = \"other\"\n",
    )
    .unwrap();
    assert!(with_model.to_entry("y", &json!({}), false).is_err());
}

/// The branch runs with the target's tool iteration budget.
#[tokio::test]
async fn the_fork_budget_limits_the_branch() {
    let env = Env::new();
    let mut spec = ui_spec(analyze_entry());
    let b: BehaviorConfig = toml::from_str(
        "[meta]\nname = \"analyze\"\n[entry]\nmode = \"fork\"\n[budget]\nmax_tool_iterations = 1\n",
    )
    .unwrap();
    spec.prompt.frozen = Some(FrozenPrompt {
        behaviors: [("analyze".to_string(), b)].into_iter().collect(),
        ..Default::default()
    });
    let sd = env.create_work(spec).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), from_bob("go", None, None));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "go" })),
        1 => tool_call("s1", "shell", json!({ "command": "echo one" })),
        2 if has_tool_result(req, "f1").is_none() => {
            tool_call("s2", "shell", json!({ "command": "echo two >> two.log" }))
        }
        _ => text("caller goes on"),
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert!(!sd.path().join("two.log").exists(), "the second iteration never ran");
    assert_eq!(sd.state().unwrap().turns_completed, 1);
}


/// V06: what is not a message stays with the caller: an event arriving while
/// the user is asked never enters the branch; it is the caller's next input.
#[tokio::test]
async fn an_event_while_asking_stays_for_the_caller() {
    let env = Env::new();
    let mut spec = ui_spec(analyze_entry());
    spec.subscriptions.push(Subscription {
        id: "s2".into(),
        mode: SubscriptionMode::Active,
        source: SubscriptionSource::ObjectEvent {
            object: "https://cam/01".into(),
            event: String::new(),
        },
        watch: vec![],
    });
    let sd = env.create_work(spec).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("go", None, None));
    let llm = ScriptedLlm::new(move |req, n| {
        let all = render(&req.messages);
        match n {
            0 => tool_call("f1", "try_analyze", json!({ "request": "go" })),
            1 => ask("q1", "'Which?'"),
            2 => {
                assert!(!all.contains("camera moved"), "the branch never sees it:\n{all}");
                assert_eq!(answer_of(req, "q1").text, "this one");
                text("analysis done")
            }
            3 => {
                assert!(!all.contains("camera moved"), "the batch continues first");
                text("done")
            }
            4 => {
                assert!(last_user_text(req).contains("camera moved"), "{all}");
                text("noted")
            }
            _ => panic!("unexpected call {n}"),
        }
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    post_blocking(
        &env.queue_dir,
        &q,
        event("cam:1", Some("s2"), "object", "https://cam/01", Some(1), "camera moved"),
    );
    post_blocking(&env.queue_dir, &q, from_bob("this one", None, None));
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 5);
    assert_eq!(sd.state().unwrap().turns_completed, 2);
}

/// xllm cannot resolve `userinput:`: it refuses to take over a run waiting
/// for the user; the session still completes the call.
#[tokio::test]
async fn xllm_refuses_a_run_waiting_for_the_user() {
    use agent_tool::xllm::{ResumeLimits, RunStore, XllmDeps, XllmRun};
    let env = Env::new();
    let sd = env.create_work(ui_spec(analyze_entry())).await;
    let q = queue_of(&sd);
    post_blocking(&env.queue_dir, &q, from_bob("go", None, None));
    let llm = ScriptedLlm::new(move |_req, n| match n {
        0 => tool_call("f1", "try_analyze", json!({ "request": "go" })),
        1 => ask("q1", "'Which?'"),
        2 => text("analysis done"),
        3 => text("done"),
        _ => panic!("unexpected call {n}"),
    });
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let other = ScriptedLlm::new(|_, _| text("never"));
    let deps = XllmDeps::default().with_llm(other.clone());
    let r = XllmRun::resume(&RunStore::disk(sd.runs_dir()), Some(&run_id), None, ResumeLimits::default(), deps).await;
    assert!(r.is_err(), "xllm must refuse: {r:?}");
    assert_eq!(other.count(), 0);
    post_blocking(&env.queue_dir, &q, from_bob("that", None, None));
    drive(&sd, &deps_with(&env, llm.clone(), &sink, &tasks), StopWhen::Idle).await;
    assert_eq!(llm.count(), 4);
}
