//! Helpers of the plan session tests: the Jarvis package of this repository
//! as the AgentRoot, a host that drives the plan and its Todos' work
//! sessions, and readers of what a scripted model was shown.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{AiContent, AiResponse, AiRole};
use libopendan::host::{run_session, HostDeps, RunOutcome};
use libopendan::runner::{BehaviorAssembler, RunnerOptions, StopSignal, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::todos::{store_of, TodoList};
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

use super::*;

pub const PACKAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/jarvis_agent/agent");

pub fn copy_dir(from: &Path, to: &Path) {
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
pub fn install_jarvis(env: &Env) {
    let pkg = Path::new(PACKAGE);
    for f in ["agent.toml", "role.md", "self.md"] {
        std::fs::copy(pkg.join(f), env.agent_root.join(f)).unwrap();
    }
    for d in ["behaviors", "cmd_skills"] {
        copy_dir(&pkg.join(d), &env.agent_root.join(d));
    }
}

/// What the tool processes (`agent-session`) find in their environment.
pub fn tool_env(env: &Env) {
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    std::env::remove_var("LIBOPENDAN_FAULT");
    std::env::remove_var(libopendan::todos::ENV_MAX_CHILDREN);
}

pub fn host(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>) -> HostDeps {
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
        notifier: None,
    }
}

pub async fn drive_with(h: &HostDeps, sd: &SessionDir) -> RunOutcome {
    Box::pin(run_session(h, sd, StopWhen::Finished, StopSignal::default(), false)).await
}

pub async fn drive_to_end(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>, sd: &SessionDir) -> RunOutcome {
    drive_with(&host(env, llm), sd).await
}

/// A plan session as `create-worksession --class plan_simple` makes it.
pub fn plan_spec(env: &Env, objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("plan_simple", Some(&env.agent_root))
        .unwrap()
        .spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.freeze = true;
    spec
}

pub fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plan,
    Work,
    Explore,
}

pub fn role(req: &LlmInferenceRequest) -> Role {
    let s = system_of(req);
    if s.contains("You are the plan session of one objective") {
        Role::Plan
    } else if s.contains("You are an exploration of a plan session") {
        Role::Explore
    } else {
        assert!(s.contains("You are a work session for one bounded task"), "{s}");
        Role::Work
    }
}

/// The Todo a work session executes (`T01` …), from its first message.
pub fn todo_of(req: &LlmInferenceRequest) -> String {
    let first = user_texts(req).into_iter().next().unwrap_or_default();
    let at = first.find("Todo T").unwrap_or_else(|| panic!("not a Todo's work session: {first}"));
    first[at + 5..at + 8].to_string()
}

/// A shell call with an intent.
pub fn shell(id: &str, intent: &str, command: &str) -> AiResponse {
    tool_call(id, "shell", json!({ "intent": intent, "command": command }))
}

pub fn calls(items: &[(&str, &str, Value)]) -> AiResponse {
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

/// Every JSON object in a tool result (the command's stdout), in order.
pub fn jsons_in(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let mut it = serde_json::Deserializer::from_str(&rest[start..]).into_iter::<Value>();
        match it.next() {
            Some(Ok(v)) => {
                out.push(v);
                rest = &rest[start + it.byte_offset()..];
            }
            _ => rest = &rest[start + 1..],
        }
    }
    out
}

/// The first JSON object in a tool result.
pub fn json_in(text: &str) -> Value {
    jsons_in(text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no JSON in {text}"))
}

pub fn list_of(path: &Path) -> TodoList {
    store_of(&SessionDir::open(path).unwrap()).load().unwrap()
}

/// Text of the user messages after the last assistant message.
pub fn after_results(req: &LlmInferenceRequest) -> Vec<String> {
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
pub fn tail_roles(req: &LlmInferenceRequest) -> Vec<AiRole> {
    let last_assistant = req
        .messages
        .iter()
        .rposition(|m| m.role == AiRole::Assistant)
        .unwrap_or(0);
    req.messages[last_assistant + 1..].iter().map(|m| m.role).collect()
}

pub fn background(req: &LlmInferenceRequest) -> Option<String> {
    after_results(req)
        .into_iter()
        .find(|t| t.starts_with("<<background_context>>"))
}

/// The work sessions a Todo's attempts ran in.
pub fn children_of(list: &TodoList, id: &str) -> Vec<String> {
    list.get(id)
        .unwrap()
        .attempts
        .iter()
        .filter_map(|a| a.child_sid.clone())
        .collect()
}

/// The end of work session `sid` came in as this session's input.
pub fn ended(req: &LlmInferenceRequest, sid: &str) -> bool {
    user_texts(req).iter().any(|u| {
        u.contains(&format!("source=\"session:{sid}\"")) && u.contains("event=\"finished\"")
    })
}

pub fn plan_result(req: &LlmInferenceRequest, id: &str) -> Option<Value> {
    has_tool_result(req, id).map(|r| json_in(&r))
}

/// The newest user message (the input batch the request answers).
pub fn last_input(req: &LlmInferenceRequest) -> String {
    user_texts(req).into_iter().last().unwrap_or_default()
}
