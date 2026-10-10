//! Library side of xagent (xAgent §10): session templates, behaviors frozen
//! from the agent's catalog, the Turn return condition, bootstrap inputs of
//! a session without a queue, sub sessions, Agent State implementations.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::AiRole;
use common::*;
use libopendan::api::{create_session, create_sub_session, SubSessionSpec};
use libopendan::host::{run_session, HostDeps};
use libopendan::protocol::*;
use libopendan::runner::{
    drive, BehaviorAssembler, DriveResult, RunnerDeps, RunnerOptions, StopSignal, StopWhen,
};
use libopendan::state::{
    AgentStateClient, ForwardingStateClient, MemBehaviorCatalog, WithBehaviors,
};
use libopendan::{InputChannel, SessionDir, SessionTemplate};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use serde_json::{json, Value};

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn template_spec(class: &str, objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load(class, None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec
}

fn behavior_llm_context() -> Value {
    json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } })
}

/// Deps of a host that takes behaviors from the agent's catalog.
fn frozen_deps(env: &Env, agent: Arc<dyn AgentStateClient>, llm: Arc<dyn LlmClient>) -> RunnerDeps {
    let mut deps = env.deps(llm);
    deps.agent = agent;
    deps.assembler = Arc::new(BehaviorAssembler::default());
    deps
}

fn write(path: &std::path::Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// E13: the work template is one Turn without an input queue. Its first
/// input is persisted with the configuration, consumed through the same
/// receipts, and survives a restart between creation and the first drive.
#[tokio::test]
async fn work_template_runs_one_turn_without_a_queue() {
    let env = Env::new();
    let mut spec = template_spec("work", "summarize the note");
    assert_eq!(spec.input_channel, Some(InputChannel::None));
    spec.prompt.initial_inputs = vec![msg("the note: buy milk"), msg("and eggs")];
    let sd = env.create_work(spec).await;
    let cfg = sd.config().unwrap();
    assert!(cfg.channels.kmsg().is_none(), "no queue although the host has a client");
    assert_eq!(cfg.session.policy.wait_user_msg, WaitPolicy::FinishFailed);
    assert_eq!(cfg.prompt.initial_inputs.len(), 2);
    // Nothing can be posted later.
    let agent = env.agent();
    let err = libopendan::post_input(agent.as_ref(), sd.sid(), &msg("late"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no input queue"), "{err}");

    let llm = ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        assert!(u.contains("buy milk") && u.contains("and eggs"), "{u}");
        assert!(u.contains("hook=\"on_init\""), "{u}");
        text("milk and eggs")
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    match &r {
        DriveResult::TurnClosed { turn, status, answer, .. } => {
            assert_eq!((*turn, *status), (1, TurnStatus::Completed));
            assert_eq!(answer.as_deref(), Some("milk and eggs"));
        }
        other => panic!("{other:?}"),
    }
    let st = sd.state().unwrap();
    assert!(st.is_finished());
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    let src = st.source(BOOTSTRAP_SRC);
    assert!(src.is_consumed(1) && src.is_consumed(2), "{src:?}");
    assert_eq!(llm.count(), 1);
    // Nothing left: no Turn closed by this drive, the session is finished.
    let again = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(again.is_finished(), "{again:?}");
}

/// E13: a `single` consumption policy takes the bootstrap inputs one by one;
/// the second one is not dropped because the bootstrap is done.
#[tokio::test]
async fn bootstrap_inputs_are_consumed_one_by_one() {
    let env = Env::new();
    let mut spec = template_spec("work", "handle each input");
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    spec.prompt.input.mode = InputMode::Single;
    spec.prompt.initial_inputs = vec![msg("first"), msg("second")];
    let sd = env.create_work(spec).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        s2.lock().unwrap().push(last_user_text(req));
        text("ok")
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    assert!(!sd.state().unwrap().source(BOOTSTRAP_SRC).is_consumed(2));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 2, .. }), "{r:?}");
    let seen = seen.lock().unwrap();
    assert!(seen[0].contains("first") && !seen[0].contains("second"), "{seen:?}");
    assert!(seen[1].contains("second") && seen[1].contains("hook=\"on_input\""), "{seen:?}");
    assert!(sd.state().unwrap().is_finished());
}

/// §4.7 / E13: nobody answers in a work session — `WAIT_USER_MSG` fails the
/// Turn with `needs_user_input` and the question is the report.
#[tokio::test]
async fn work_template_cannot_wait_for_the_user() {
    let env = Env::new();
    let mut spec = template_spec("work", "pick a color");
    spec.prompt.llm_context = behavior_llm_context();
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| {
        text("<response><report><![CDATA[which color?]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(
        matches!(r, DriveResult::TurnClosed { status: TurnStatus::Failed, .. }),
        "{r:?}"
    );
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Failed));
    assert_eq!(st.last_error.as_ref().unwrap()["kind"], "needs_user_input");
    assert!(sd.report().unwrap().contains("which color?"));
}

/// E6: where waiting is allowed the Turn stays open; `TurnClosed` reports
/// the open Turn as it is, and the answer continues the same Turn.
#[tokio::test]
async fn turn_closed_reports_the_open_turn_and_then_its_result() {
    let env = Env::new();
    let mut spec = template_spec("ui", "chat");
    spec.prompt.llm_context = behavior_llm_context();
    let sd = env.create_work(spec).await;
    let agent = env.agent();
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>"),
        _ => {
            assert!(render(&req.messages).contains("blue"));
            text("<response><report><![CDATA[blue it is]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        }
    });
    let deps = env.deps(llm.clone());
    // Nothing to do and no Turn: never a made-up open Turn.
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("pick a color for me"))
        .await
        .unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    match &r {
        DriveResult::TurnOpen { turn, waiting_for, .. } => {
            assert_eq!(*turn, 1);
            assert_eq!(waiting_for.as_ref().unwrap().kind, WaitingKind::Input);
        }
        other => panic!("{other:?}"),
    }
    assert!(sd.state().unwrap().open_turn.is_some());
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("blue")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    match &r {
        DriveResult::TurnClosed { turn, status, answer, .. } => {
            assert_eq!((*turn, *status), (1, TurnStatus::Completed));
            assert_eq!(answer.as_deref(), Some("blue it is"));
        }
        other => panic!("{other:?}"),
    }
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "one Turn, counted once");
    assert!(!st.is_finished());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
}

const PLAN: &str = r#"
[meta]
objective = "plan the work"
next = ["do"]

[prompt]
mode = "behavior"
system = "PLAN-V1 for {{ session.id }} as {{ behavior.name }}. __INCLUDE(./rules.inc)__"
"#;

const DO: &str = r#"
[prompt]
mode = "behavior"
system = "DO-V1"

[entry]
mode = "create_sub_context"

[budget]
max_tool_iterations = 7
"#;

const REVIEW: &str = r#"
[prompt]
mode = "behavior"
system = "REVIEW-V1"

[entry]
mode = "switch_context"
"#;

fn write_catalog(env: &Env) {
    write(&env.agent_root.join("role.md"), "I am Jarvis.");
    write(&env.agent_root.join("behaviors/plan.toml"), PLAN);
    write(&env.agent_root.join("behaviors/rules.inc"), "RULES-V1");
    write(&env.agent_root.join("behaviors/do.toml"), DO);
    write(&env.agent_root.join("behaviors/review.toml"), REVIEW);
}

fn frozen_spec(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = template_spec("ui", objective);
    spec.prompt.llm_context = behavior_llm_context();
    spec.prompt.behavior = Some("plan".into());
    spec.freeze = true;
    spec
}

/// E2: a session freezes its behaviors when it is constructed. Changing the
/// catalog afterwards reaches new sessions only; a behavior used for the
/// first time later is frozen then, from the catalog as it is at that time.
#[tokio::test]
async fn behaviors_are_frozen_with_the_session() {
    let env = Env::new();
    write_catalog(&env);
    let sd = env.create_work(frozen_spec("frozen")).await;
    let cfg = sd.config().unwrap();
    let frozen = cfg.prompt.frozen.as_ref().expect("frozen at creation");
    assert_eq!(frozen.identity.role, "I am Jarvis.");
    assert_eq!(
        frozen.behaviors.keys().cloned().collect::<Vec<_>>(),
        vec!["do".to_string(), "plan".to_string()],
        "entry + declared closure, not the whole catalog"
    );
    let entries = cfg.behaviors().unwrap();
    assert_eq!(entries["do"].mode, ContextMode::CreateSubContext);
    assert_eq!(entries["do"].llm_context["max_tool_iterations"], 7);
    assert_eq!(entries["plan"].mode, ContextMode::SwitchContext);

    // The agent edits its behaviors.
    write(
        &env.agent_root.join("behaviors/plan.toml"),
        &PLAN.replace("PLAN-V1", "PLAN-V2"),
    );
    write(&env.agent_root.join("role.md"), "I am someone else.");
    write(
        &env.agent_root.join("behaviors/review.toml"),
        &REVIEW.replace("REVIEW-V1", "REVIEW-V2"),
    );
    let systems = Arc::new(Mutex::new(Vec::new()));
    let s2 = systems.clone();
    let llm = ScriptedLlm::new(move |req, n| {
        s2.lock().unwrap().push(system_of(req));
        match n {
            0 => text("<response><report><![CDATA[one]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>"),
            1 => text("<response><next_behavior>review</next_behavior></response>"),
            _ => text("<response><report><![CDATA[reviewed]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>"),
        }
    });
    let agent = env.agent();
    let deps = frozen_deps(&env, agent.clone(), llm.clone());
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("go")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("again")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    let systems = systems.lock().unwrap();
    assert!(systems[0].contains("I am Jarvis.") && !systems[0].contains("someone else"));
    assert!(
        systems[0].contains(&format!("PLAN-V1 for {} as plan. RULES-V1", sd.sid())),
        "{}",
        systems[0]
    );
    // E11: the same behavior's next run has the same system prefix.
    assert_eq!(systems[0], systems[1]);
    // `review` was not in the closure: frozen on first use, as it is now.
    assert!(systems[2].contains("REVIEW-V2"), "{}", systems[2]);
    let cfg = sd.config().unwrap();
    assert!(cfg.prompt.frozen.as_ref().unwrap().behaviors.contains_key("review"));
    assert!(read_worklog(&sd).iter().any(|e| matches!(
        &e.body,
        WorklogBody::ControlApplied { command, detail, .. }
            if command == "behavior_frozen" && detail["behaviors"] == json!(["review"])
    )));

    // A new session takes the catalog as it is now.
    let sd2 = env.create_work(frozen_spec("second")).await;
    let f2 = sd2.config().unwrap().prompt.frozen.unwrap();
    assert_eq!(f2.identity.role, "I am someone else.");
    assert!(f2.behaviors["plan"].prompt.system.as_ref().unwrap().contains("PLAN-V2"));
}

/// E19 (configuration side): a target without an entry mode is refused when
/// the session is frozen — there is no fallback mode.
#[tokio::test]
async fn a_behavior_without_entry_mode_cannot_be_frozen() {
    let env = Env::new();
    write_catalog(&env);
    write(
        &env.agent_root.join("behaviors/do.toml"),
        "[prompt]\nsystem = \"DO\"\n",
    );
    let agent = env.agent();
    let ch = env.channels();
    let err = create_session(&env.app_dir, frozen_spec("bad"), agent.as_ref(), APP, ch.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("declares no entry mode"), "{err}");
    // A fork target with its own system is refused as well.
    write(
        &env.agent_root.join("behaviors/do.toml"),
        "[prompt]\nsystem = \"DO\"\n[entry]\nmode = \"fork\"\n",
    );
    let err = create_session(&env.app_dir, frozen_spec("bad"), agent.as_ref(), APP, ch.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("fork keeps the caller's system"), "{err}");
}

/// E12: without frozen behaviors and without a readable catalog nothing is
/// guessed and nothing is inferred.
#[tokio::test]
async fn missing_frozen_behaviors_block_the_drive() {
    let env = Env::new();
    let mut spec = frozen_spec("no catalog");
    spec.freeze = false;
    let sd = env.create_work(spec).await;
    assert!(sd.config().unwrap().prompt.frozen.is_none());
    let llm = ScriptedLlm::new(|_, _| panic!("no inference"));
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("go")).await.unwrap();
    let r = drive(&sd, &frozen_deps(&env, agent.clone(), llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::RecoveryBlocked(_)), "{r:?}");
    assert_eq!(llm.count(), 0);
    // The catalog appears: the driver freezes at its first drive.
    write_catalog(&env);
    let llm = ScriptedLlm::new(|req, _| {
        assert!(system_of(req).contains("PLAN-V1"));
        text("<response><report><![CDATA[ok]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
    });
    let r = drive(&sd, &frozen_deps(&env, agent, llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    let cfg = sd.config().unwrap();
    assert!(cfg.prompt.frozen.is_some());
    assert_eq!(cfg.config_rev, 2);
}

/// E4: the runner only depends on the `AgentStateClient` trait. The same
/// scenario gives the same result on the file client, on a client whose
/// behaviors live in the host process, and through the forwarding stand-in
/// of the kRPC client (which exposes no AgentRoot path).
#[tokio::test]
async fn agent_state_implementations_behave_alike() {
    async fn scenario(env: &Env, agent: Arc<dyn AgentStateClient>) -> (String, SessionState, RegistryEntry) {
        let ch = env.channels();
        let mut spec = SessionTemplate::load("work", None).unwrap().spec("say hi");
        spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
        spec.prompt.behavior = Some("plan".into());
        spec.prompt.initial_inputs = vec![msg("hello")];
        spec.freeze = true;
        let sd = create_session(&env.app_dir, spec, agent.as_ref(), APP, ch.as_ref())
            .await
            .unwrap();
        let system = Arc::new(Mutex::new(String::new()));
        let s2 = system.clone();
        let llm = ScriptedLlm::new(move |req, _| {
            *s2.lock().unwrap() = system_of(req);
            text("hi")
        });
        let deps = frozen_deps(env, agent.clone(), llm);
        let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(
            matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }),
            "{r:?}"
        );
        let entry = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
        let system = system.lock().unwrap().clone();
        (system.replace(sd.sid(), "<sid>"), sd.state().unwrap(), entry)
    }
    let simple = "[prompt]\nsystem = \"SIMPLE for {{ session.id }}\"\n";

    let fs_env = Env::new();
    write(&fs_env.agent_root.join("role.md"), "I am Jarvis.");
    write(&fs_env.agent_root.join("behaviors/plan.toml"), simple);
    let (fs_system, fs_state, fs_entry) = scenario(&fs_env, fs_env.agent()).await;

    let mem_env = Env::new();
    let catalog = MemBehaviorCatalog::new(IdentityText {
        role: "I am Jarvis.".into(),
        ..Default::default()
    });
    catalog.put("plan", toml::from_str(simple).unwrap());
    let in_process: Arc<dyn AgentStateClient> =
        Arc::new(WithBehaviors::new(mem_env.agent(), Arc::new(catalog)));
    libopendan::state::register_in_process(in_process.clone());
    let connected = libopendan::state::connect(AGENT, APP, Default::default())
        .await
        .unwrap();
    libopendan::state::unregister_in_process(AGENT);
    let (mem_system, mem_state, mem_entry) = scenario(&mem_env, connected).await;

    let fwd_env = Env::new();
    write(&fwd_env.agent_root.join("role.md"), "I am Jarvis.");
    write(&fwd_env.agent_root.join("behaviors/plan.toml"), simple);
    let forwarding: Arc<dyn AgentStateClient> = Arc::new(ForwardingStateClient::new(fwd_env.agent()));
    assert!(forwarding.agent_root().is_none());
    let (fwd_system, fwd_state, fwd_entry) = scenario(&fwd_env, forwarding).await;

    assert!(fs_system.contains("SIMPLE for <sid>") && fs_system.contains("I am Jarvis."));
    assert_eq!(fs_system, mem_system);
    assert_eq!(fs_system, fwd_system);
    for (st, e) in [(&mem_state, &mem_entry), (&fwd_state, &fwd_entry)] {
        assert_eq!(
            (st.run_state, st.outcome, st.turn_seq, st.turns_completed, st.bootstrap_done),
            (
                fs_state.run_state,
                fs_state.outcome,
                fs_state.turn_seq,
                fs_state.turns_completed,
                fs_state.bootstrap_done
            )
        );
        assert_eq!(e.status.run_state, fs_entry.status.run_state);
        assert_eq!(e.status.outcome, fs_entry.status.outcome);
        assert_eq!(e.status.report_brief.replace(&e.session_id, ""), fs_entry.status.report_brief.replace(&fs_entry.session_id, ""));
    }
}

/// `spawn({objective, report, wait})`: what `agent-session create-worksession`
/// does, as an in-process tool (the CLI form is covered by the CLI tests).
struct Spawn {
    agent: Arc<dyn AgentStateClient>,
    channels: Arc<libopendan::channel::KmsgChannels>,
    parent: String,
}

#[async_trait]
impl AgentTool for Spawn {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn".into(),
            description: "create a sub session".into(),
            args_schema: json!({ "type": "object", "properties": {
                "objective": { "type": "string" }, "report": { "type": "string" },
                "wait": { "type": "boolean" }, "key": { "type": "string" },
                "interactive": { "type": "boolean" }, "post_to": { "type": "string" },
                "behavior": { "type": "string" },
                "parent_objective": { "type": "string" },
                "text": { "type": "string" } } }),
            output_schema: json!({ "type": "object" }),
            usage: None,
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        // `post`: answer a sub session that waits for input.
        if let Some(to) = args.get("post_to").and_then(Value::as_str) {
            let input = msg(text("text"));
            self.agent
                .sessions()
                .post_input(to, &input)
                .await
                .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
            return Ok(AgentToolResult::from_details(json!({ "posted": to })).with_tool("spawn"));
        }
        let sub = SubSessionSpec {
            objective: text("objective"),
            report: serde_json::from_value(json!(text("report"))).unwrap_or_default(),
            key: Some(text("key")),
            interactive: args.get("interactive").and_then(Value::as_bool).unwrap_or(false),
            behavior: args.get("behavior").and_then(Value::as_str).map(str::to_string),
            ..Default::default()
        };
        let parent = if let Some(objective) = args.get("parent_objective").and_then(Value::as_str) {
            self.agent
                .sessions()
                .query(&RegistryQuery::default())
                .await
                .map_err(|error| AgentToolError::ExecFailed(error.to_string()))?
                .into_iter()
                .find(|entry| entry.objective == objective)
                .map(|entry| entry.session_id)
                .ok_or_else(|| AgentToolError::ExecFailed("parent objective not found".into()))?
        } else {
            self.parent.clone()
        };
        let sd = create_sub_session(
            self.agent.as_ref(),
            APP,
            &parent,
            sub,
            self.channels.as_ref(),
        )
        .await
        .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
        let mut r = AgentToolResult::from_details(json!({ "session_id": sd.sid(), "status": "created" }))
            .with_tool("spawn")
            .with_status(if wait { AgentToolStatus::Pending } else { AgentToolStatus::Success });
        if wait {
            r.task_id = Some(format!("session:{}", sd.sid()));
        }
        r.summary = format!("sub session {}", sd.sid());
        Ok(r)
    }
}

fn spawn_host(env: &Env, llm: Arc<dyn LlmClient>, parent: &SessionDir) -> HostDeps {
    let agent: Arc<dyn AgentStateClient> = env.agent();
    let mut xllm = agent_tool::xllm::XllmDeps::default().with_llm(llm);
    xllm.host_tools.insert(
        "spawn".into(),
        Arc::new(Spawn {
            agent: agent.clone(),
            channels: env.channels(),
            parent: parent.sid().to_string(),
        }),
    );
    HostDeps {
        who: APP.into(),
        agent,
        inputs: env.channels(),
        waker: Arc::new(libopendan::channel::PollWaker),
        xllm,
        assembler: Arc::new(libopendan::runner::DefaultAssembler::default()),
        session_cli: None,
        app_tools: Vec::new(),
        options: RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait: Duration::from_secs(20),
            load_hints: false,
            ..Default::default()
        },
        max_child_concurrency: 4,
        bridges: Vec::new(),
        outbound: None,
        turn_tasks: None,
        runtime_id: None,
    }
}

fn parent_spec(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = template_spec("work", objective);
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [ { "groupname": "bash" }, { "name": "spawn" } ] }
    });
    spec
}

fn is_child(req: &LlmInferenceRequest) -> bool {
    system_of(req).contains("child work")
}

/// E21: a parent without a queue hands work to two sub sessions and ends
/// its run. The session does not finish while it must hear from them: the
/// Turn stays open, their ends arrive as input of that same Turn, and the
/// parent concludes. Each sub session has its own lease, Turn and run; the
/// same creating call never makes a second one.
#[tokio::test]
async fn a_parent_waits_for_its_reporting_sub_sessions() {
    let env = Env::new();
    let parent = env.create_work(parent_spec("delegate and summarize")).await;
    let llm = ScriptedLlm::new(|req, _| {
        if is_child(req) {
            return text(&format!("child result of {}", if system_of(req).contains("child work A") { "A" } else { "B" }));
        }
        let u = render(&req.messages);
        if u.contains("event=\"finished\"") {
            assert!(u.contains("child result of A") || u.contains("child result of B"), "{u}");
            return text("summary of both");
        }
        if has_tool_result(req, "s2").is_some() {
            return text("delegated, done for now");
        }
        if has_tool_result(req, "s1").is_some() {
            return tool_call("s2", "spawn", json!({ "objective": "child work B", "report": "final", "key": "b" }));
        }
        tool_call("s1", "spawn", json!({ "objective": "child work A", "report": "final", "key": "a" }))
    });
    let host = spawn_host(&env, llm.clone(), &parent);
    let out = run_session(&host, &parent, StopWhen::TurnClosed, StopSignal::default(), false).await;
    match &out.result {
        DriveResult::TurnClosed { turn, status, answer, .. } => {
            assert_eq!((*turn, *status), (1, TurnStatus::Completed));
            assert_eq!(answer.as_deref(), Some("summary of both"));
        }
        other => panic!("{other:?}"),
    }
    assert!(out.children.is_empty(), "{:?}", out.children);
    let st = parent.state().unwrap();
    assert!(st.is_finished());
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "the Turn stayed open for the children");
    let wl = read_worklog(&parent);
    assert!(
        wl.iter().any(|e| matches!(&e.body, WorklogBody::InputBatch { inputs, .. }
            if inputs.iter().any(|i| i.src == INTERNAL_CHILD_SRC))),
        "the children's end joined the open Turn"
    );
    let agent = env.agent();
    let children = agent.sessions().children_of(&[parent.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 2);
    for c in &children {
        assert_eq!(c.status.run_state, RunState::Finished);
        assert_eq!(c.origin.as_ref().unwrap().report, Some(ReportMode::Final));
        let sd = SessionDir::open(&c.location).unwrap();
        let cs = sd.state().unwrap();
        assert_eq!((cs.turn_seq, cs.turns_completed), (1, 1), "its own Turn");
        assert!(sd.config().unwrap().channels.kmsg().is_none());
        assert!(sd.binding_opt().unwrap().is_some(), "bound by its own first drive");
    }
    // The same call again: the same sub session.
    let again = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "child work A".into(),
            key: Some("a".into()),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    assert!(children.iter().any(|c| c.session_id == again.sid()));
    assert_eq!(
        agent.sessions().children_of(&[parent.sid().to_string()]).await.unwrap().len(),
        2
    );
}

/// E21 (`--wait`): the creating call is suspended on `session:<sid>` and
/// answered from the registry when the sub session ends — as that call's
/// tool result, not as an event as well.
#[tokio::test]
async fn a_waited_sub_session_answers_the_suspended_call() {
    let env = Env::new();
    let parent = env.create_work(parent_spec("delegate and wait")).await;
    let llm = ScriptedLlm::new(|req, _| {
        if is_child(req) {
            return text("forty-two");
        }
        match has_tool_result(req, "s1") {
            Some(r) => {
                assert!(r.contains("forty-two") && r.contains("\"status\":\"finished\""), "{r}");
                assert!(!render(&req.messages).contains("event=\"finished\""));
                text("the child said forty-two")
            }
            None => tool_call("s1", "spawn", json!({ "objective": "child work", "report": "final", "wait": true, "key": "w" })),
        }
    });
    let host = spawn_host(&env, llm.clone(), &parent);
    let out = run_session(&host, &parent, StopWhen::TurnClosed, StopSignal::default(), false).await;
    assert!(
        matches!(&out.result, DriveResult::TurnClosed { status: TurnStatus::Completed, answer, .. }
            if answer.as_deref() == Some("the child said forty-two")),
        "{:?}",
        out.result
    );
    let st = parent.state().unwrap();
    assert!(st.is_finished());
    assert_eq!(llm.count(), 3, "parent twice, child once");
    let wl = read_worklog(&parent);
    assert!(
        !wl.iter().any(|e| matches!(&e.body, WorklogBody::InputBatch { inputs, .. } | WorklogBody::TurnStarted { inputs, .. }
            if inputs.iter().any(|i| i.src == INTERNAL_CHILD_SRC))),
        "the end was consumed as the tool result"
    );
}

/// §4.11: the number of unfinished sub sessions and their nesting are
/// bounded by the parent's policy; `report = none` children are not waited
/// for.
#[tokio::test]
async fn sub_session_limits_and_unreported_children() {
    let env = Env::new();
    let mut spec = template_spec("work", "limits");
    spec.policy.max_sub_sessions = 1;
    spec.policy.max_session_depth = 1;
    let parent = env.create_work(spec).await;
    let agent = env.agent();
    let ch = env.channels();
    let sub = |key: &str, report| SubSessionSpec {
        objective: "child work".into(),
        key: Some(key.into()),
        report,
        ..Default::default()
    };
    let c1 = create_sub_session(agent.as_ref(), APP, parent.sid(), sub("1", ReportMode::None), ch.as_ref())
        .await
        .unwrap();
    let err = create_sub_session(agent.as_ref(), APP, parent.sid(), sub("2", ReportMode::Final), ch.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("max_sub_sessions"), "{err}");
    let err = create_sub_session(agent.as_ref(), APP, c1.sid(), sub("3", ReportMode::Final), ch.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("max_session_depth"), "{err}");
    let ccfg = c1.config().unwrap();
    assert_eq!(ccfg.session.origin.as_ref().unwrap().parent_session.as_deref(), Some(parent.sid()));
    assert_eq!(ccfg.session.driver.principal, APP);
    assert!(
        ccfg.workspace.is_none(),
        "an unbound child uses its own SessionDir"
    );
    // The parent finishes without waiting for a child that does not report.
    let llm = ScriptedLlm::new(|_, _| text("done alone"));
    let r = drive(&parent, &env.deps(llm), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{r:?}");
    assert!(parent.state().unwrap().is_finished());
}

/// E22: stopping a parent stops the sub sessions its host drives — one
/// without a queue through the driver's own stop signal.
#[tokio::test]
async fn a_stopped_parent_stops_its_sub_sessions() {
    let env = Env::new();
    let mut spec = parent_spec("delegate, then get stopped");
    spec.input_channel = Some(InputChannel::Queue);
    let parent = env.create_work(spec).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&parent));
    let llm = ScriptedLlm::new(move |req, _| {
        if is_child(req) {
            // The child works on something long.
            return tool_call("c1", "shell", json!({ "command": "sleep 30" }));
        }
        if has_tool_result(req, "s1").is_some() {
            post_blocking(
                &qd,
                &q,
                PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None }),
            );
            return tool_call("p1", "shell", json!({ "command": "sleep 30" }));
        }
        tool_call("s1", "spawn", json!({ "objective": "child work", "report": "final", "key": "s" }))
    });
    let host = spawn_host(&env, llm.clone(), &parent);
    let started = std::time::Instant::now();
    let out = run_session(&host, &parent, StopWhen::TurnClosed, StopSignal::default(), false).await;
    assert!(
        matches!(out.result, DriveResult::TurnClosed { status: TurnStatus::Stopped, .. }),
        "{:?}",
        out.result
    );
    assert!(started.elapsed() < Duration::from_secs(25), "nobody waited for the sleeps");
    assert_eq!(parent.state().unwrap().outcome, Some(Outcome::Stopped));
    let agent = env.agent();
    let children = agent.sessions().children_of(&[parent.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].status.outcome, Some(Outcome::Stopped), "{:?}", children[0].status);
    assert!(out.children.is_empty());
}

/// §4.4 / E28: a stop requested by the driving process itself (SIGINT) is
/// the same stop: the running tool is interrupted, the Turn closes as
/// stopped — also for a session without a queue.
#[tokio::test]
async fn the_drivers_own_stop_signal_stops_the_session() {
    let env = Env::new();
    let sd = env.create_work(template_spec("work", "long command")).await;
    let llm = ScriptedLlm::new(|_, _| tool_call("c1", "shell", json!({ "command": "sleep 30" })));
    let mut deps = env.deps(llm);
    let stop = StopSignal::default();
    deps.stop = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        stop.request();
    });
    let started = std::time::Instant::now();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { status: TurnStatus::Stopped, .. }), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Stopped));
    assert!(read_worklog(&sd).iter().any(|e| matches!(
        &e.body,
        WorklogBody::ControlApplied { command, .. } if command == "stop"
    )));
}

/// E14: semi-subscription state that arrives while a run executes is shown
/// right before the hand-over input of the context being entered — one
/// receipt, the snapshot first — and only that version is cleared.
#[tokio::test]
async fn semi_snapshot_precedes_a_context_switch_input() {
    let env = Env::new();
    let mut spec = work_spec("plan, then review");
    spec.prompt.llm_context = behavior_llm_context();
    spec.prompt.behavior = Some("plan".into());
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "review": { "mode": "switch_context" } } }),
    );
    spec.subscriptions.push(Subscription {
        id: "watch".into(),
        mode: SubscriptionMode::Semi,
        source: SubscriptionSource::ObjectEvent {
            object: "doc".into(),
            event: String::new(),
        },
        watch: Vec::new(),
    });
    let sd = env.create_work(spec).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            post_blocking(&qd, &q, event("doc:7", Some("watch"), "object", "doc", Some(7), "doc is at v7"));
            text("<response><next_behavior>review</next_behavior></response>")
        }
        _ => {
            let users = user_texts(req);
            let (snapshot, input) = (&users[users.len() - 2], &users[users.len() - 1]);
            assert!(
                snapshot.starts_with("<semi_subscription_snapshot>") && snapshot.contains("doc is at v7"),
                "{snapshot}"
            );
            assert!(input.contains("hook=\"on_context_switch\""), "{input}");
            text("<response><report end=\"true\"><![CDATA[reviewed]]></report></response>")
        }
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.pending_events.iter().all(|p| p.latest.is_none() && p.terminal.is_none()) || st.pending_events.is_empty());
    let switched = read_worklog(&sd)
        .into_iter()
        .find_map(|e| match e.body {
            WorklogBody::InputBatch { hook, events, .. }
                if hook.as_deref() == Some("on_context_switch") =>
            {
                Some(events)
            }
            _ => None,
        })
        .expect("a hand-over batch");
    assert_eq!(switched, vec!["doc:7".to_string()], "the snapshot is part of the hand-over receipt");
}

/// E22: an interactive sub session asks its parent. The parent hears
/// `needs_input` as an input of its open Turn and answers through the
/// child's queue; the child goes on with the answer and its end is then
/// reported like any other.
#[tokio::test]
async fn an_interactive_sub_session_asks_its_parent() {
    let env = Env::new();
    let parent = env.create_work(parent_spec("delegate a question")).await;
    let llm = ScriptedLlm::new(|req, _| {
        let all = render(&req.messages);
        if is_child(req) {
            // Behavior loop of the child: ask, then deliver.
            return if all.contains("the color is blue") {
                text("<response><report end=\"true\"><![CDATA[painted blue]]></report></response>")
            } else {
                text("<response><report><![CDATA[which color?]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
            };
        }
        let u = last_user_text(req);
        if u.contains("event=\"finished\"") {
            assert!(u.contains("painted blue"), "{u}");
            return text("the child painted it blue");
        }
        if has_tool_result(req, "a1").is_some() {
            return text("answered the child");
        }
        if u.contains("event=\"needs_input\"") {
            assert!(u.contains("which color?"), "{u}");
            let child = u
                .split("source=\"session:")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap()
                .to_string();
            return tool_call("a1", "spawn", json!({ "post_to": child, "text": "the color is blue" }));
        }
        if has_tool_result(req, "s1").is_some() {
            return text("asked a child to paint");
        }
        tool_call("s1", "spawn", json!({ "objective": "child work: paint", "report": "final", "interactive": true, "behavior": "painter", "key": "i" }))
    });
    // The child's behavior comes from the agent's catalog: a behavior loop,
    // so it can wait for input.
    write(
        &env.agent_root.join("behaviors/painter.toml"),
        "[prompt]\nmode = \"behavior\"\nsystem = \"You paint things.\"\n",
    );
    let mut host = spawn_host(&env, llm.clone(), &parent);
    host.assembler = Arc::new(BehaviorAssembler::default());
    host.options.max_wait = Duration::from_secs(30);
    let agent = env.agent();
    let out = run_session(&host, &parent, StopWhen::TurnClosed, StopSignal::default(), false).await;
    assert!(
        matches!(&out.result, DriveResult::TurnClosed { status: TurnStatus::Completed, answer, .. }
            if answer.as_deref() == Some("the child painted it blue")),
        "{:?}",
        out.result
    );
    let st = parent.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let children = agent.sessions().children_of(&[parent.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 1);
    let child = SessionDir::open(&children[0].location).unwrap();
    let cs = child.state().unwrap();
    // The question was delivered as a report: that reply completed the
    // child's first Turn (D2); the parent's answer is its second.
    assert_eq!((cs.turn_seq, cs.turns_completed), (2, 2));
    assert_eq!(cs.outcome, Some(Outcome::Succeeded));
    assert!(child.config().unwrap().channels.kmsg().is_some(), "interactive: it has a queue");
}

/// E21 (`--report progress`) / §4.15: progress of a sub session is kept as
/// semi-subscription state and shown before the next controlled input; what
/// needs attention is an input; and a parent that would finish while a
/// reporting sub session is unsettled keeps its Turn open waiting for it.
#[tokio::test]
async fn progress_is_observed_and_the_parent_waits_for_children() {
    let env = Env::new();
    write(
        &env.agent_root.join("behaviors/painter.toml"),
        "[prompt]\nmode = \"behavior\"\nsystem = \"You paint things.\"\n",
    );
    let agent: Arc<dyn AgentStateClient> = env.agent();
    let mut spec = template_spec("work", "delegate");
    spec.prompt.initial_inputs = vec![msg("go")];
    let parent = env.create_work(spec).await;
    let child = create_sub_session(
        agent.as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: "child work: paint".into(),
            report: ReportMode::Progress,
            interactive: true,
            behavior: Some("painter".into()),
            key: Some("p".into()),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    // The child asks without delivering anything: its Turn stays open.
    let child_llm = ScriptedLlm::new(|_, _| {
        text("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>")
    });
    let r = drive(
        &child,
        &frozen_deps(&env, agent.clone(), child_llm),
        StopWhen::Idle,
    )
    .await;
    assert!(
        matches!(
            r,
            DriveResult::Idle {
                run_state: RunState::Waiting,
                ..
            }
        ),
        "{r:?}"
    );
    let entry = agent.sessions().lookup(child.sid()).await.unwrap().unwrap();
    assert_eq!(entry.status.waiting_for, Some(WaitingKind::Input));
    assert!(entry.status.turn_open);

    let csid = child.sid().to_string();
    let llm = ScriptedLlm::new(move |req, _| {
        let users = user_texts(req);
        let (snapshot, input) = (&users[users.len() - 2], &users[users.len() - 1]);
        assert!(
            snapshot.contains("<semi_subscription_snapshot>")
                && snapshot.contains(&format!("sub session {csid} is waiting")),
            "{snapshot}"
        );
        assert!(
            input.contains("event=\"needs_input\"") && input.contains(&format!("source=\"session:{csid}\"")),
            "{input}"
        );
        text("I will come back to it")
    });
    let mut deps = env.deps(llm.clone());
    deps.options.max_wait = Duration::from_millis(300);
    let r = drive(&parent, &deps, StopWhen::TurnClosed).await;
    match &r {
        DriveResult::TurnOpen { turn, waiting_for, .. } => {
            assert_eq!(*turn, 1);
            let w = waiting_for.as_ref().unwrap();
            assert_eq!(w.kind, WaitingKind::Children);
            assert_eq!(w.refs, vec![child.sid().to_string()]);
        }
        other => panic!("{other:?}"),
    }
    let st = parent.state().unwrap();
    assert!(!st.is_finished() && st.open_turn.is_some() && st.live_run.is_none());
    assert_eq!(llm.count(), 1);
    // The same state is not delivered twice.
    let r = drive(&parent, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnOpen { .. }), "{r:?}");
    assert_eq!(llm.count(), 1);
}

/// §4.6: the timer bridge only produces events through the registry; the
/// session's subscription decides that they are inputs.
#[tokio::test]
async fn the_timer_bridge_posts_subscribed_events() {
    use libopendan::bridge::{EventBridge, TimerBridge};
    let env = Env::new();
    let mut spec = template_spec("ui", "tick");
    spec.subscriptions.push(Subscription {
        id: "tick".into(),
        mode: SubscriptionMode::Active,
        source: SubscriptionSource::Timer { name: "beat".into() },
        watch: Vec::new(),
    });
    spec.extensions
        .insert("opendan".into(), json!({ "timers": { "beat": { "every_secs": 1 } } }));
    let sd = env.create_work(spec).await;
    let agent: Arc<dyn AgentStateClient> = env.agent();
    let bridge = TimerBridge { agent: agent.clone(), who: APP.into() };
    let cfg = sd.config().unwrap();
    assert!(bridge.wants(&cfg));
    let task = tokio::spawn(async move { bridge.run(cfg).await });
    tokio::time::sleep(Duration::from_millis(2300)).await;
    task.abort();
    let llm = ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        assert!(u.contains("source=\"timer:beat\"") && u.contains("event=\"fired\""), "{u}");
        text("tock")
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(
        matches!(
            r,
            DriveResult::TurnClosed {
                status: TurnStatus::Completed,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn a_waited_child_hands_the_workspace_back_to_its_parent() {
    let env = Env::new();
    let directory = env.root.join("shared-project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = parent_spec("delegate within one workspace and wait");
    spec.workspace = Some(env.workspace(&directory).await);
    let parent = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| {
        if is_child(req) {
            if has_tool_result(req, "child-write").is_some() {
                return text("child updated the project");
            }
            return tool_call(
                "child-write",
                "shell",
                json!({"command":"echo child > handover.txt"}),
            );
        }
        if has_tool_result(req, "parent-write").is_some() {
            return text("parent completed the project");
        }
        match has_tool_result(req, "delegate") {
            Some(result) => {
                assert!(result.contains("child updated the project"), "{result}");
                tool_call(
                    "parent-write",
                    "shell",
                    json!({"command":"echo parent >> handover.txt"}),
                )
            }
            None => tool_call(
                "delegate",
                "spawn",
                json!({
                    "objective":"child work", "report":"final", "wait":true, "key":"handover",
                }),
            ),
        }
    });
    let mut host = spawn_host(&env, llm.clone(), &parent);
    host.options.max_wait = Duration::from_millis(100);
    let parent_deps = host.runner_deps(&parent, StopSignal::default()).unwrap();
    let waiting = drive(&parent, &parent_deps, StopWhen::TurnClosed).await;
    assert!(
        matches!(waiting, DriveResult::TurnOpen { .. }),
        "{waiting:?}"
    );
    let children = host
        .agent
        .sessions()
        .children_of(&[parent.sid().into()])
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    let child = SessionDir::open(&children[0].location).unwrap();
    assert_eq!(
        parent.config().unwrap().workspace,
        child.config().unwrap().workspace
    );
    let child_deps = host.runner_deps(&child, StopSignal::default()).unwrap();
    let lease = parent
        .acquire(parent_deps.holder())
        .unwrap()
        .into_result("parent")
        .unwrap();
    assert!(matches!(
        drive(&child, &child_deps, StopWhen::Finished).await,
        DriveResult::Busy { .. }
    ));
    let run_id = parent.state().unwrap().live_run.unwrap().run_id;
    let original = parent.runs().record(&run_id).unwrap();
    let mut unsafe_record = original.clone();
    let mut unresolved = unsafe_record.inflight[0].clone();
    unresolved.call_id = "unresolved-background-write".into();
    unresolved.tool = "shell".into();
    unresolved.args = "{}".into();
    unsafe_record.inflight.push(unresolved);
    {
        let _run_lock = parent.runs().try_lock(&run_id).unwrap().unwrap();
        parent.runs().store().write_record(&unsafe_record).unwrap();
    }
    drop(lease);
    assert!(matches!(
        drive(&child, &child_deps, StopWhen::Finished).await,
        DriveResult::Busy { .. }
    ));
    assert_eq!(llm.count(), 1);
    {
        let _lease = parent
            .acquire(parent_deps.holder())
            .unwrap()
            .into_result("parent")
            .unwrap();
        let _run_lock = parent.runs().try_lock(&run_id).unwrap().unwrap();
        parent.runs().store().write_record(&original).unwrap();
    }
    let child_result = drive(&child, &child_deps, StopWhen::Finished).await;
    assert!(
        matches!(
            child_result,
            DriveResult::Finished {
                outcome: Some(Outcome::Succeeded),
                ..
            }
        ),
        "{child_result:?}"
    );
    let result = drive(&parent, &parent_deps, StopWhen::TurnClosed).await;
    assert!(
        matches!(
            result,
            DriveResult::TurnClosed {
                status: TurnStatus::Completed,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("handover.txt")).unwrap(),
        "child\nparent\n"
    );
    assert_eq!(llm.count(), 5);
}

#[tokio::test]
async fn the_host_completes_a_workspace_delegation_chain_without_wait_timeouts() {
    let env = Env::new();
    let directory = env.root.join("shared-project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = parent_spec("root work");
    spec.workspace = Some(env.workspace(&directory).await);
    let parent = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| {
        let system = system_of(req);
        if system.contains("grandchild work") {
            return if has_tool_result(req, "write-c").is_some() {
                text("grandchild finished")
            } else {
                tool_call(
                    "write-c",
                    "shell",
                    json!({"command":"echo grandchild > handover.txt"}),
                )
            };
        }
        let (delegate, write, objective, command, answer) = if system.contains("child work") {
            (
                "delegate-c",
                "write-b",
                "grandchild work",
                "echo child >> handover.txt",
                "child finished",
            )
        } else {
            (
                "delegate-b",
                "write-a",
                "child work",
                "echo parent >> handover.txt",
                "parent finished",
            )
        };
        if has_tool_result(req, write).is_some() {
            return text(answer);
        }
        if has_tool_result(req, delegate).is_some() {
            return tool_call(write, "shell", json!({"command":command}));
        }
        tool_call(
            delegate,
            "spawn",
            json!({"objective":objective,"report":"final","wait":true,"key":delegate,
                "parent_objective": if system.contains("child work") { Some("child work") } else { None },
            }),
        )
    });
    let mut host = spawn_host(&env, llm.clone(), &parent);
    host.options.max_wait = RunnerOptions::default().max_wait;
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run_session(
            &host,
            &parent,
            StopWhen::TurnClosed,
            StopSignal::default(),
            false,
        ),
    )
    .await;
    let result = result.expect("workspace handoff must not wait for max_wait");
    assert!(
        matches!(
            result.result,
            DriveResult::TurnClosed {
                status: TurnStatus::Completed,
                ..
            }
        ),
        "{result:?}"
    );
    assert!(result.children.is_empty(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(directory.join("handover.txt")).unwrap(),
        "grandchild\nchild\nparent\n"
    );
    assert_eq!(llm.count(), 8);
}

#[tokio::test]
async fn stopping_a_workspace_handoff_interrupts_the_child_and_parent() {
    let env = Env::new();
    let directory = env.root.join("shared-project");
    std::fs::create_dir(&directory).unwrap();
    let mut spec = parent_spec("wait for child until stopped");
    spec.workspace = Some(env.workspace(&directory).await);
    let parent = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| {
        if is_child(req) {
            return tool_call(
                "child-long-write",
                "shell",
                json!({"command":"touch child-started; sleep 30"}),
            );
        }
        tool_call(
            "delegate",
            "spawn",
            json!({"objective":"child work","wait":true,"report":"final","key":"cancel"}),
        )
    });
    let mut host = spawn_host(&env, llm, &parent);
    host.options.max_wait = RunnerOptions::default().max_wait;
    let stop = StopSignal::default();
    let child_stop = stop.clone();
    let driven = parent.clone();
    let run = tokio::spawn(async move {
        run_session(&host, &driven, StopWhen::TurnClosed, child_stop, false).await
    });
    for _ in 0..100 {
        if directory.join("child-started").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(directory.join("child-started").exists());
    stop.request();
    let result = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .expect("stop cannot wait for the handoff timeout")
        .unwrap();
    assert!(
        matches!(
            result.result,
            DriveResult::TurnClosed {
                status: TurnStatus::Stopped,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(parent.state().unwrap().outcome, Some(Outcome::Stopped));
    assert!(result.children.is_empty(), "{result:?}");
}
