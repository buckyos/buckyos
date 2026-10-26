//! plan_simple (notepads/opendan-plan-simple-todo.md §8.1): one Turn of a
//! function call plan that runs every Todo in a sub session of its own,
//! with an `intent` on every call and the background after tool results.
//!
//! The Jarvis package of this repository is installed as the AgentRoot
//! (the real `plan_simple`, `work_simple` and `explore` behaviors and the
//! `[session.plan_simple]` class); the models are scripted, everything else
//! is real: `agent-session` is the `xagent` binary, sub sessions are driven
//! by the host's ChildDriver, the list, registry and worklogs are the
//! committed files.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::{AiContent, AiResponse, AiRole};
use common::*;
use libopendan::host::{run_session, HostDeps, RunOutcome};
use libopendan::protocol::*;
use libopendan::runner::{BehaviorAssembler, RunnerOptions, StopSignal, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::todos::{store_of, ReviewState, TodoList, TodoMode, TodoStatus};
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

const PACKAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/jarvis_agent/agent");

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// The Jarvis package as the AgentRoot.
fn install_jarvis(env: &Env) {
    let pkg = Path::new(PACKAGE);
    for f in ["agent.toml", "role.md", "self.md"] {
        std::fs::copy(pkg.join(f), env.agent_root.join(f)).unwrap();
    }
    for d in ["behaviors", "cmd_skills"] {
        copy_dir(&pkg.join(d), &env.agent_root.join(d));
    }
}

fn tool_env(env: &Env) {
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    std::env::remove_var("LIBOPENDAN_FAULT");
    std::env::remove_var(libopendan::todos::ENV_MAX_CHILDREN);
}

fn host(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>) -> HostDeps {
    let agent: Arc<dyn AgentStateClient> = env.agent();
    HostDeps {
        who: APP.into(),
        agent,
        inputs: env.channels(),
        waker: Arc::new(libopendan::channel::PollWaker),
        xllm: agent_tool::xllm::XllmDeps::default().with_llm(llm),
        assembler: Arc::new(BehaviorAssembler::default()),
        session_cli: Some(PathBuf::from(env!("CARGO_BIN_EXE_xagent"))),
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        options: RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait: Duration::from_secs(60),
            load_hints: false,
            ..Default::default()
        },
        max_child_concurrency: 4,
        bridges: Vec::new(),
        outbound: None,
        objects: None,
        turn_tasks: None,
        notify: None,
        share: None,
        runtime_id: None,
    }
}

async fn drive_to_end(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>, sd: &SessionDir) -> RunOutcome {
    let h = host(env, llm);
    Box::pin(run_session(&h, sd, StopWhen::Finished, StopSignal::default(), false)).await
}

/// A plan session as `create-worksession --class plan_simple` makes it.
fn plan_spec(env: &Env, objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("plan_simple", Some(&env.agent_root))
        .unwrap()
        .spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.freeze = true;
    spec
}

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plan,
    Work,
    Explore,
}

fn role(req: &LlmInferenceRequest) -> Role {
    let s = system_of(req);
    if s.contains("You are the plan session of one objective") {
        Role::Plan
    } else if s.contains("You are an exploration sub session") {
        Role::Explore
    } else {
        assert!(s.contains("You are a work session for one bounded task"), "{s}");
        Role::Work
    }
}

/// A shell call with an intent.
fn shell(id: &str, intent: &str, command: &str) -> AiResponse {
    tool_call(id, "shell", json!({ "intent": intent, "command": command }))
}

fn calls(items: &[(&str, &str, Value)]) -> AiResponse {
    let content = items
        .iter()
        .map(|(id, name, args)| {
            AiContent::tool_use(
                *id,
                *name,
                args.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            )
        })
        .collect();
    AiResponse::new(buckyos_api::AiMessage::new(AiRole::Assistant, content))
}

/// The first JSON object in a tool result (the command's stdout).
fn json_in(text: &str) -> Value {
    let start = text.find('{').unwrap_or_else(|| panic!("no JSON in {text}"));
    serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<Value>()
        .next()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn list_of(path: &Path) -> TodoList {
    store_of(&SessionDir::open(path).unwrap()).load().unwrap()
}

/// Text of the user messages after the last assistant message (what the
/// host appended after the latest tool results).
fn after_results(req: &LlmInferenceRequest) -> Vec<String> {
    let last_assistant = req
        .messages
        .iter()
        .rposition(|m| m.role == AiRole::Assistant)
        .unwrap_or(0);
    req.messages[last_assistant..]
        .iter()
        .filter(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
        .collect()
}

/// Roles of the messages after the last assistant message.
fn tail_roles(req: &LlmInferenceRequest) -> Vec<AiRole> {
    let last_assistant = req
        .messages
        .iter()
        .rposition(|m| m.role == AiRole::Assistant)
        .unwrap_or(0);
    req.messages[last_assistant + 1..].iter().map(|m| m.role).collect()
}

fn background(req: &LlmInferenceRequest) -> Option<String> {
    after_results(req)
        .into_iter()
        .find(|t| t.starts_with("<<background_context>>"))
}

/// The child sessions a Todo's attempts ran in.
fn children_of(list: &TodoList, id: &str) -> Vec<String> {
    list.get(id)
        .unwrap()
        .attempts
        .iter()
        .filter_map(|a| a.child_sid.clone())
        .collect()
}

#[derive(Default)]
struct Seen {
    /// Plan requests: (step, background block after its latest results).
    backgrounds: Vec<(String, Option<String>)>,
    notes: Vec<String>,
}

/// V-config: the real package's class and behavior. The model receives
/// `intent` as a required argument of every function (the strict report
/// included) and no XML action / DO hand-over protocol; the child behaviors
/// keep their own contract.
#[tokio::test]
async fn the_plan_simple_class_and_its_function_contract() {
    let env = Env::new();
    install_jarvis(&env);
    let t = SessionTemplate::load("plan_simple", Some(&env.agent_root)).unwrap();
    assert_eq!(t.kind, SessionKind::Work);
    assert_eq!(t.completion, CompletionPolicy::ExplicitReport);
    assert_eq!(t.default_behavior.as_deref(), Some("plan_simple"));
    assert_eq!(t.ask_user_timeout_secs, 10800);
    let work = SessionTemplate::load("work", Some(&env.agent_root)).unwrap();
    assert_eq!(work.default_behavior.as_deref(), Some("plan"), "the work class is unchanged");
    let b = env.agent().behaviors().get("plan_simple").await.unwrap().unwrap();
    let entry = b.to_entry("plan_simple", &json!({}), true).unwrap();
    assert!(entry.call_intent);
    assert_eq!(entry.input.unwrap().background, InputBackground::AfterTools);
    for child in ["work_simple", "explore"] {
        let c = env.agent().behaviors().get(child).await.unwrap().unwrap();
        assert!(!c.capabilities.call_intent, "{child} keeps its own contract");
    }
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "check the contract")).await;
    let specs: Arc<Mutex<Vec<llm_context::deps::ToolSpecLite>>> = Arc::default();
    let sys = Arc::new(Mutex::new(String::new()));
    let (s2, y2) = (specs.clone(), sys.clone());
    let llm = ScriptedLlm::new(move |req, _| {
        if s2.lock().unwrap().is_empty() {
            *s2.lock().unwrap() = req.tool_specs.clone();
            *y2.lock().unwrap() = system_of(req);
        }
        tool_call("r", "report", json!({ "intent": "nothing to plan: finish", "report": "nothing to do", "is_end": true }))
    });
    let out = drive_to_end(&env, llm, &sd).await;
    assert!(matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }), "{:?}", out.result);
    let specs = specs.lock().unwrap().clone();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"shell") && names.contains(&"report"), "{names:?}");
    for s in &specs {
        let required = s.args_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "intent"), "{}: {}", s.name, s.args_schema);
        assert_eq!(s.args_schema["properties"]["intent"]["type"], "string", "{}", s.name);
    }
    let report = specs.iter().find(|s| s.name == "report").unwrap();
    assert_eq!(report.args_schema["additionalProperties"], false, "the report stays strict");
    let sys = sys.lock().unwrap().clone();
    assert!(sys.contains("carries `intent`"), "the runtime protocol states the contract");
    assert!(!sys.contains("<next_behavior>") && !sys.contains("<actions>"), "no XML protocol: {sys}");
    let frozen = sd.config().unwrap();
    assert!(frozen.behaviors().unwrap().get("do").is_none(), "no DO hand-over target");
}

/// V-serial / V-intent / V-wait / V-background / V-workspace / V-turn: the
/// plan corrects calls without intent (nothing executed), adds two serial
/// Todos, is refused the second before the first is accepted, runs each in
/// a work session of its own that changes the inherited workspace while
/// the plan waits (no deadlock over the workspace), reads each report right
/// after the wait, reviews it, is refused an early end and finishes with a
/// report — all in one Turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_serial_todos_in_sub_sessions_in_one_turn() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let project = env.root.join("project");
    std::fs::create_dir(&project).unwrap();
    let mut spec = plan_spec(&env, "fix the greeting and verify it");
    spec.workspace = Some(env.workspace(&project).await);
    let sd = env.create_work(spec).await;
    let path = sd.path().to_path_buf();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s2 = seen.clone();
    let p2 = path.clone();
    let project2 = project.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => panic!("no explorations here"),
        Role::Work => {
            let first = user_texts(req).into_iter().next().unwrap_or_default();
            let todo = if first.contains("Todo T01") { "T01" } else { "T02" };
            assert!(first.contains("Acceptance criteria"), "the full task: {first}");
            if todo == "T02" {
                assert!(first.contains("Builds on these reviewed results"), "{first}");
                assert!(first.contains("read-session"), "{first}");
            }
            // The child's calls carry no intent: its contract is its own.
            if has_tool_result(req, "w1").is_none() {
                let cmd = if todo == "T01" {
                    "echo 'hello, world' > \"$OPENDAN_WORKSPACE_DIR/greeting.txt\" && cat \"$OPENDAN_WORKSPACE_DIR/greeting.txt\""
                } else {
                    "grep -c hello \"$OPENDAN_WORKSPACE_DIR/greeting.txt\" > check.txt && cat check.txt"
                };
                return tool_call("w1", "shell", json!({ "command": cmd }));
            }
            let out = has_tool_result(req, "w1").unwrap();
            tool_call(
                "w2",
                "report",
                json!({ "report": format!("{todo}: done\n\nEVIDENCE-{todo}: {out}"), "artifacts": if todo == "T02" { json!(["check.txt"]) } else { json!([]) }, "is_end": true }),
            )
        }
        Role::Plan => {
            let step = |id: &str| has_tool_result(req, id);
            let bg = background(req);
            let mut s = s2.lock().unwrap();
            let last = req
                .messages
                .iter()
                .rev()
                .find_map(|m| m.content.iter().find_map(|c| match c {
                    AiContent::ToolResult { call_id, .. } => Some(call_id.clone()),
                    _ => None,
                }))
                .unwrap_or_default();
            s.backgrounds.push((last.clone(), bg.clone()));
            drop(s);
            let list = list_of(&p2);
            if step("p0").is_none() {
                // Without intent: refused before anything runs.
                return tool_call("p0", "shell", json!({ "command": "touch NO_INTENT" }));
            }
            if step("p1").is_none() {
                assert!(step("p0").unwrap().contains("`intent` is missing"), "{:?}", step("p0"));
                assert!(!p2.join("NO_INTENT").exists(), "a call without intent never runs");
                // A batch: one blank intent, one fine — each checked alone.
                return calls(&[
                    ("p1", "shell", json!({ "intent": "  ", "command": "touch BLANK" })),
                    ("p1b", "shell", json!({ "intent": 7, "command": "touch NUMBER" })),
                    ("p1c", "shell", json!({ "intent": "see the working directory", "command": "pwd" })),
                ]);
            }
            if step("p2").is_none() {
                assert!(step("p1").unwrap().contains("`intent` is blank"));
                assert!(step("p1b").unwrap().contains("must be a string"));
                assert!(!step("p1c").unwrap().contains("intent"), "{:?}", step("p1c"));
                assert!(!p2.join("BLANK").exists() && !p2.join("NUMBER").exists());
                return shell(
                    "p2",
                    "Record the fix as the first serial Todo.",
                    "agent-session todo add \"fix greeting\" --accept \"greeting.txt says hello\" --scope greeting.txt --content - <<'EOF'\nWrite greeting.txt in the workspace with the text `hello, world`.\nEOF",
                );
            }
            if step("p3").is_none() {
                let r = json_in(&step("p2").unwrap());
                assert_eq!(r["todo_id"], "T01");
                s2.lock().unwrap().notes.push(r["note"].as_str().unwrap().to_string());
                return shell(
                    "p3",
                    "Record the verification, after T01 is accepted.",
                    "agent-session todo add \"verify greeting\" --after T01 --accept \"a count of greetings\" --content \"Count the greetings in greeting.txt of the workspace.\"",
                );
            }
            if step("p4").is_none() {
                return shell("p4", "Try T02 early: it must wait for T01.", "agent-session todo dispatch T02");
            }
            if step("p5").is_none() {
                let r = json_in(&step("p4").unwrap());
                assert_eq!(r["dispatched"], false, "{r}");
                assert!(r["reason"].as_str().unwrap().contains("runs after T01"), "{r}");
                return shell("p5", "Start T01 in a work session of its own.", "agent-session todo dispatch T01");
            }
            let a = || json_in(&step("p5").unwrap())["child_sid"].as_str().unwrap().to_string();
            if step("p6").is_none() {
                let r = json_in(&step("p5").unwrap());
                assert_eq!((r["mode"].as_str(), r["dispatched"].as_bool()), (Some("serial"), Some(true)), "{r}");
                assert!(r["note"].as_str().unwrap().contains("agent-session wait"), "{r}");
                return shell("p6", "A repeated dispatch finds the same sub session.", "agent-session todo dispatch T01");
            }
            if step("p7").is_none() {
                assert_eq!(json_in(&step("p6").unwrap())["child_sid"].as_str().unwrap(), a());
                return shell("p7", "Wait for T01's report.", &format!("agent-session wait {}", a()));
            }
            if step("p8").is_none() {
                assert!(step("p7").unwrap().contains("finished"), "{:?}", step("p7"));
                return shell("p8", "Read T01's full report.", "agent-session todo show T01 --report");
            }
            if step("p9").is_none() {
                let r = json_in(&step("p8").unwrap());
                assert!(r["report"]["report"].as_str().unwrap_or_default().contains("EVIDENCE-T01"), "readable right after the wait: {r}");
                assert_eq!(r["review"], "pending");
                return shell(
                    "p9",
                    "Check the workspace file before accepting T01.",
                    &format!("cat {}/greeting.txt", project2.display()),
                );
            }
            if step("p10").is_none() {
                assert!(step("p9").unwrap().contains("hello, world"));
                return shell("p10", "Accept T01: the file holds the greeting.", "agent-session todo review T01 --accept --reason \"greeting.txt says hello, world\"");
            }
            if step("p11").is_none() {
                // An end while T02 is pending is refused, correctably.
                return tool_call("p11", "report", json!({ "intent": "Try to finish early.", "report": "early", "is_end": true }));
            }
            if step("p12").is_none() {
                assert!(step("p11").unwrap().contains("not converged"), "{:?}", step("p11"));
                return shell("p12", "T01 is accepted: start T02.", "agent-session todo dispatch T02");
            }
            let b = || json_in(&step("p12").unwrap())["child_sid"].as_str().unwrap().to_string();
            if step("p13").is_none() {
                assert_ne!(b(), a(), "every Todo has its own sub session");
                return shell("p13", "Wait for T02's verification.", &format!("agent-session wait {}", b()));
            }
            if step("p14").is_none() {
                // Two calls in one batch: the background follows both.
                return calls(&[
                    ("p14", "shell", json!({ "intent": "Read T02's report.", "command": "agent-session todo show T02 --report" })),
                    ("p14b", "shell", json!({ "intent": "Accept T02 with its count.", "command": "agent-session todo review T02 --accept --reason \"the count is 1\"" })),
                ]);
            }
            assert!(list.unconverged().is_empty(), "{:?}", list.unconverged());
            tool_call(
                "p15",
                "report",
                json!({ "intent": "Both Todos are accepted: deliver.", "report": "greeting fixed and verified (T01, T02)", "is_end": true }),
            )
        }
    });
    let out = drive_to_end(&env, llm.clone(), &sd).await;
    let st = sd.state().unwrap();
    assert!(
        matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }),
        "{:?} {:?} {:?}",
        out.result,
        st.last_error,
        sd.state().unwrap().one_line_status
    );
    // One Turn for the whole plan.
    let log = read_worklog(&sd);
    let k = kinds(&log);
    assert_eq!(k.iter().filter(|x| **x == "turn_started").count(), 1, "{k:?}");
    assert_eq!(k.iter().filter(|x| **x == "turn_ended").count(), 1, "{k:?}");
    assert!(!k.contains(&"step"), "no behavior steps / DO");
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    // Every call kept its intent in the record.
    let mut recorded = 0;
    for e in &log {
        if let WorklogBody::AssistantMessage { tool_calls, .. } = &e.body {
            for c in tool_calls {
                if !["p0", "p1", "p1b"].contains(&c.call_id.as_str()) {
                    assert!(c.args["intent"].as_str().is_some_and(|i| !i.trim().is_empty()), "{c:?}");
                }
                recorded += 1;
            }
        }
    }
    assert!(recorded >= 17, "{recorded}");
    // The list: two serial Todos, each attempt in its own work session.
    let list = list_of(&path);
    for id in ["T01", "T02"] {
        let t = list.get(id).unwrap();
        assert_eq!(t.mode, TodoMode::Serial);
        assert_eq!((t.status, t.review_state()), (TodoStatus::Completed, ReviewState::Accepted), "{id}");
        assert_eq!(t.attempts.len(), 1, "{id}: one attempt");
        assert_eq!(t.attempts[0].behavior.as_deref(), Some("work_simple"));
        assert_eq!(t.behavior.as_deref(), Some("work_simple"));
        assert!(t.attempts[0].workspace_id.is_some(), "{id} inherited the workspace");
    }
    let (a, b) = (children_of(&list, "T01"), children_of(&list, "T02"));
    assert_eq!((a.len(), b.len()), (1, 1));
    assert_ne!(a, b);
    let agent = env.agent();
    let children = agent.sessions().children_of(&[sd.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 2, "no duplicate sub session");
    for c in &children {
        let cd = SessionDir::open(&c.location).unwrap();
        let cst = cd.state().unwrap();
        assert_eq!(cst.outcome, Some(Outcome::Succeeded));
        assert_eq!(cst.turns_completed, 1, "a child has its own Turn");
    }
    assert_eq!(std::fs::read_to_string(project.join("greeting.txt")).unwrap(), "hello, world\n");
    // The note of a function call plan: dispatch, not DO.
    let seen = seen.lock().unwrap();
    assert!(seen.notes[0].contains("todo dispatch T01"), "{:?}", seen.notes);
    // Background: T01 appears when added; nothing is repeated when nothing
    // changed (a repeated dispatch, the early end's refusal is shown with
    // no line); after the wait the completed attempt with its pending
    // review; after the batch of two calls one block, after both results.
    let bg = |after: &str| {
        seen.backgrounds
            .iter()
            .find(|(c, _)| c == after)
            .unwrap_or_else(|| panic!("no plan request after {after}"))
            .1
            .clone()
    };
    assert!(bg("p2").unwrap().contains("id=\"T01\""), "{:?}", bg("p2"));
    assert!(bg("p0").is_none(), "nothing changed: {:?}", bg("p0"));
    let after_wait = bg("p7").expect("a background after the wait");
    assert!(after_wait.contains("id=\"T01\"") && after_wait.contains("status=\"completed\"") && after_wait.contains("review=\"pending\""), "{after_wait}");
    assert!(bg("p6").is_none(), "a repeated dispatch changes nothing: {:?}", bg("p6"));
    let last = llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Plan).last().cloned().unwrap();
    let roles = tail_roles(&last);
    assert_eq!(roles.last(), Some(&AiRole::User), "{roles:?}");
    assert!(
        roles.len() >= 2 && roles[..roles.len() - 1].iter().all(|r| *r == AiRole::Tool),
        "the observation follows every result of its batch: {roles:?}"
    );
    let block = background(&last).unwrap();
    assert!(block.contains("id=\"T02\"") && block.contains("review=\"accepted\""), "{block}");
}

fn plan_result(req: &LlmInferenceRequest, id: &str) -> Option<Value> {
    has_tool_result(req, id).map(|r| json_in(&r))
}

#[derive(Default)]
struct Explorations {
    /// Exploration children: (marker, first inference, report) times.
    starts: Vec<(String, std::time::Instant)>,
    ends: Vec<(String, std::time::Instant)>,
    /// The plan waited for T02's sub session.
    waited_t02: bool,
}

/// V-branches / V-retry / V-settle: two explorations run in parallel; a
/// rejected result is retried (a new attempt in a new sub session with the
/// same runtime, workspace and behavior, the old attempt and report kept);
/// the other exploration is never waited for — its end, shown after tool
/// results, counts as heard, so the final report is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn parallel_explorations_a_retry_and_ends_heard_from_the_background() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "investigate two leads")).await;
    let path = sd.path().to_path_buf();
    let ex = Arc::new(Mutex::new(Explorations::default()));
    let x2 = ex.clone();
    let p2 = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => panic!("no serial work here"),
        Role::Explore => {
            let first = user_texts(req).into_iter().next().unwrap_or_default();
            let (marker, slow) = if first.contains("LEAD-A") { ("A", true) } else { ("B", false) };
            let attempt2 = first.contains("attempt 2");
            if has_tool_result(req, "x1").is_none() {
                x2.lock().unwrap().starts.push((marker.into(), std::time::Instant::now()));
                let pause = if slow { "sleep 0.5 && " } else { "" };
                return tool_call("x1", "shell", json!({ "command": format!("{pause}echo FOUND-{marker} > find.txt && cat find.txt") }));
            }
            x2.lock().unwrap().ends.push((marker.into(), std::time::Instant::now()));
            let evidence = if marker == "A" && !attempt2 { "no reproduction".to_string() } else { format!("REPRO-{marker}") };
            tool_call("x2", "report", json!({ "report": format!("{marker}: {evidence}"), "artifacts": ["find.txt"], "is_end": true }))
        }
        Role::Plan => {
            let list = list_of(&p2);
            let r = |id: &str| plan_result(req, id);
            if has_tool_result(req, "e1").is_none() {
                return calls(&[
                    ("e1", "shell", json!({ "intent": "Probe lead A in parallel.", "command": "agent-session todo explore \"lead A\" --accept \"a reproduction\" --content \"Reproduce LEAD-A.\"" })),
                    ("e1b", "shell", json!({ "intent": "Probe lead B in parallel.", "command": "agent-session todo explore \"lead B\" --accept \"a reproduction\" --content \"Reproduce LEAD-B.\"" })),
                ]);
            }
            let child = |res: &Value| res["child_sid"].as_str().unwrap().to_string();
            if has_tool_result(req, "e2").is_none() {
                let (a, b) = (r("e1").unwrap(), r("e1b").unwrap());
                assert_eq!((a["todo_id"].as_str(), b["todo_id"].as_str()), (Some("T01"), Some("T02")), "{a} {b}");
                assert_eq!(list.explorations_in_flight().len() + list.todos.iter().filter(|t| t.latest().is_some_and(|x| x.ended())).count(), 2);
                return shell("e2", "Wait for lead A, the one the plan depends on.", &format!("agent-session wait {}", child(&a)));
            }
            if has_tool_result(req, "e3").is_none() {
                return shell("e3", "Read lead A's report.", "agent-session todo show T01 --report");
            }
            if has_tool_result(req, "e4").is_none() {
                assert!(r("e3").unwrap()["report"]["report"].as_str().unwrap().contains("no reproduction"));
                return shell("e4", "Reject A: it shows no reproduction.", "agent-session todo review T01 --reject --reason \"no reproduction shown\"");
            }
            if has_tool_result(req, "e5").is_none() {
                return shell("e5", "Retry A with a reproduction.", "agent-session todo retry T01 --reason \"show the reproduction output\"");
            }
            if has_tool_result(req, "e6").is_none() {
                let retried = r("e5").unwrap();
                assert_eq!((retried["dispatched"].as_bool(), retried["attempt"].as_u64()), (Some(true), Some(2)), "{retried}");
                return shell("e6", "Wait for A's second attempt.", &format!("agent-session wait {}", child(&retried)));
            }
            if has_tool_result(req, "e7").is_none() {
                return calls(&[
                    ("e7", "shell", json!({ "intent": "Read A's new report.", "command": "agent-session todo show T01 --report" })),
                    ("e7b", "shell", json!({ "intent": "Accept A's reproduction.", "command": "agent-session todo review T01 --accept --reason \"REPRO-A shown\"" })),
                ]);
            }
            let t02 = list.get("T02").unwrap();
            if t02.in_flight() {
                if has_tool_result(req, "e8").is_none() {
                    x2.lock().unwrap().waited_t02 = true;
                    let sid = t02.latest().unwrap().child_sid.clone().unwrap();
                    return shell("e8", "B is still running: wait for it.", &format!("agent-session wait {sid}"));
                }
            }
            if t02.review_state() == ReviewState::Pending {
                return shell("e9", "Accept B after reading it.", "agent-session todo review T02 --accept --reason \"REPRO-B shown\"");
            }
            if has_tool_result(req, "e10").is_none() {
                return tool_call("e10", "report", json!({ "intent": "Both leads are accepted: deliver.", "report": "A and B reproduce", "is_end": true }));
            }
            let res = has_tool_result(req, "e10").unwrap();
            panic!("the final report was refused: {res}");
        }
    });
    let out = drive_to_end(&env, llm.clone(), &sd).await;
    let st = sd.state().unwrap();
    assert!(
        matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }),
        "{:?} {:?}",
        out.result,
        st.last_error
    );
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let ex = ex.lock().unwrap();
    assert!(!ex.waited_t02, "B finished long before; its end came through the background");
    let start_b = ex.starts.iter().find(|(m, _)| m == "B").unwrap().1;
    let end_a = ex.ends.iter().find(|(m, _)| m == "A").unwrap().1;
    assert!(start_b < end_a, "the explorations ran in parallel");
    let list = list_of(&path);
    let t1 = list.get("T01").unwrap();
    assert_eq!(t1.attempts.len(), 2);
    let (first, second) = (&t1.attempts[0], &t1.attempts[1]);
    assert_ne!(first.child_sid, second.child_sid);
    assert_eq!(first.summary.as_deref(), Some("A: no reproduction"), "the old attempt keeps its result");
    assert!(first.report.is_some());
    assert_eq!((second.runtime_id.clone(), second.workspace_id.clone(), second.behavior.as_deref()), (first.runtime_id.clone(), first.workspace_id.clone(), Some("explore")));
    assert_eq!(second.reason.as_deref(), Some("show the reproduction output"));
    assert_eq!(t1.reviews.len(), 2);
    let children = env.agent().sessions().children_of(&[sd.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 3, "two explorations and one retry");
}

const ENV_ROOT: &str = "PLAN_SIMPLE_CRASH_ROOT";
const ENV_SESSION: &str = "PLAN_SIMPLE_CRASH_SESSION";

/// A small plan: one serial Todo, waited for, reviewed, delivered. The
/// child appends one line per execution.
fn crash_script(path: PathBuf) -> Arc<ScriptedLlm> {
    ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => panic!("no explorations"),
        Role::Work => {
            if has_tool_result(req, "w1").is_none() {
                // Slower than the plan's next boundary.
                return tool_call("w1", "shell", json!({ "command": "sleep 0.3; echo run >> ran.txt" }));
            }
            tool_call("w2", "report", json!({ "report": "T01: ran", "is_end": true }))
        }
        Role::Plan => {
            let r = |id: &str| plan_result(req, id);
            if has_tool_result(req, "q1").is_none() {
                return shell("q1", "Record the only Todo.", "agent-session todo add \"run once\" --content \"Append one line to ran.txt.\" --accept \"one line\"");
            }
            if has_tool_result(req, "q2").is_none() {
                return shell("q2", "Start T01.", "agent-session todo dispatch T01");
            }
            if has_tool_result(req, "q3").is_none() {
                let sid = r("q2").unwrap()["child_sid"].as_str().unwrap().to_string();
                return shell("q3", "Wait for T01.", &format!("agent-session wait {sid}"));
            }
            if has_tool_result(req, "q4").is_none() {
                let blocks: Vec<String> = after_results(req).into_iter().filter(|t| t.starts_with("<<background_context>>")).collect();
                assert_eq!(blocks.len(), 1, "one observation after the wait: {blocks:?}");
                assert!(blocks[0].contains("status=\"completed\""), "{blocks:?}");
                let all = user_texts(req).into_iter().filter(|t| t.contains("status=\\\"completed\\\"") || t.contains("status=\"completed\"")).count();
                assert_eq!(all, 1, "the completed attempt is shown once");
                return shell("q4", "Accept T01.", "agent-session todo review T01 --accept --reason \"it ran\"");
            }
            let _ = &path;
            tool_call("q5", "report", json!({ "intent": "Deliver.", "report": "ran once", "is_end": true }))
        }
    })
}

/// The driving child process of the crash tests (does nothing in a normal
/// test run).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plan_crash_child() {
    let Ok(root) = std::env::var(ENV_ROOT) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    let sid = std::env::var(ENV_SESSION).unwrap();
    let e = env.agent().sessions().lookup(&sid).await.unwrap().unwrap();
    let sd = SessionDir::open(&e.location).unwrap();
    let out = drive_to_end(&env, crash_script(sd.path().to_path_buf()), &sd).await;
    panic!("the crash child was expected to die, it ended with {:?}", out.result);
}

/// V-background recovery: the host dies right before the observation after
/// a wait is placed, or right after it was published. A new host shows it
/// exactly once, opens no second Turn, runs no call again and creates no
/// second sub session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_observation_survives_a_crash_before_and_after_its_commit() {
    for fault in ["observe:before_inject#3", "observe:after_inject_commit#3"] {
        let env = Env::new();
        install_jarvis(&env);
        let sd = env.create_work(plan_spec(&env, "run once")).await;
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "plan_crash_child", "--nocapture", "--test-threads=1"])
            .env(ENV_ROOT, &env.root)
            .env(ENV_SESSION, sd.sid())
            .env("LIBOPENDAN_FAULT", fault)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "{fault}: the host must die");
        let list = list_of(sd.path());
        assert_eq!(list.get("T01").unwrap().latest().unwrap().status, TodoStatus::Completed, "{fault}");
        tool_env(&env);
        let llm = crash_script(sd.path().to_path_buf());
        let out = drive_to_end(&env, llm.clone(), &sd).await;
        let st = sd.state().unwrap();
        assert!(
            matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }),
            "{fault}: {:?} {:?}",
            out.result,
            st.last_error
        );
        assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "{fault}: the same Turn");
        let k = kinds(&read_worklog(&sd));
        assert_eq!(k.iter().filter(|x| **x == "turn_started").count(), 1, "{fault}");
        let children = env.agent().sessions().children_of(&[sd.sid().to_string()]).await.unwrap();
        assert_eq!(children.len(), 1, "{fault}");
        let ran = std::fs::read_to_string(Path::new(&children[0].location).join("ran.txt")).unwrap();
        assert_eq!(ran.lines().count(), 1, "{fault}: the child ran once");
        // The recovered host only infers from where the crash left the plan.
        let plan_calls = llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Plan).count();
        assert!(plan_calls <= 3, "{fault}: {plan_calls} plan inferences after the restart");
    }
}

/// V-wait states: a wait with a deadline returns while the sub session
/// still runs (not its success); the next wait returns its failure; the
/// failed attempt is in the list right away, reviewed as a failure, and the
/// plan ends with a report that says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wait_deadline_and_a_failed_sub_session() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "try the impossible")).await;
    let path = sd.path().to_path_buf();
    let llm = ScriptedLlm::fallible(move |req, _| match role(req) {
        Role::Explore => panic!("no explorations"),
        Role::Work if has_tool_result(req, "w1").is_none() => {
            Ok(tool_call("w1", "shell", json!({ "command": "sleep 3" })))
        }
        Role::Work => {
            Err(llm_context::error::LLMComputeError::Provider {
                failure: llm_context::error::ProviderFailure::Permanent,
                message: "the input of this task is missing".into(),
            })
        }
        Role::Plan => Ok({
            let r = |id: &str| plan_result(req, id);
            let list = list_of(&path);
            if has_tool_result(req, "f1").is_none() {
                shell("f1", "Record the task.", "agent-session todo add \"impossible\" --content \"Do what cannot be done.\"")
            } else if has_tool_result(req, "f2").is_none() {
                shell("f2", "Start T01.", "agent-session todo dispatch T01")
            } else if has_tool_result(req, "f3").is_none() {
                let sid = r("f2").unwrap()["child_sid"].as_str().unwrap().to_string();
                shell("f3", "Look whether T01 ends within half a second.", &format!("agent-session wait {sid} --wait-ms 500"))
            } else if has_tool_result(req, "f4").is_none() {
                let res = has_tool_result(req, "f3").unwrap();
                assert!(!res.contains("finished") && res.contains("running"), "the deadline answers with the running state: {res}");
                assert_eq!(list.get("T01").unwrap().status, TodoStatus::Running);
                let sid = r("f2").unwrap()["child_sid"].as_str().unwrap().to_string();
                shell("f4", "Wait for T01 to end.", &format!("agent-session wait {sid}"))
            } else if has_tool_result(req, "f5").is_none() {
                let res = has_tool_result(req, "f4").unwrap();
                assert!(res.contains("finished") && res.contains("failed"), "{res}");
                let t = list.get("T01").unwrap().clone();
                assert_eq!(t.status, TodoStatus::Failed, "in the list before the next inference");
                assert!(t.latest().unwrap().error.as_deref().unwrap_or_default().contains("missing"), "{t:?}");
                let bg = background(req).expect("the failure is shown");
                assert!(bg.contains("status=\"failed\"") && bg.contains("review=\"pending\""), "{bg}");
                shell("f5", "Record that T01 failed.", "agent-session todo review T01 --reject --reason \"the task input is missing\"")
            } else {
                tool_call("f6", "report", json!({ "intent": "Report the blocker.", "report": "Not done: T01 failed because its input is missing.", "is_end": true }))
            }
        }),
    });
    let out = drive_to_end(&env, llm, &sd).await;
    let st = sd.state().unwrap();
    assert!(
        matches!(out.result, libopendan::runner::DriveResult::Finished { .. }),
        "{:?} {:?}",
        out.result,
        st.last_error
    );
    assert!(st.final_report.unwrap().report.starts_with("Not done"));
    let t = list_of(sd.path()).get("T01").unwrap().clone();
    assert_eq!((t.status, t.review_state()), (TodoStatus::Failed, ReviewState::Rejected));
}
