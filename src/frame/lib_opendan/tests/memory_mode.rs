//! How a conversation uses the Agent's Memory, as the user created it
//! (chat route TODO §4, D8 / D9): normal, no write, neither.

mod common;

use std::path::PathBuf;

use common::*;
use libopendan::memory::MemoryQuery;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::{AgentStateClient, SessionDir, SessionTemplate};
use serde_json::json;

async fn chat(env: &Env, memory: MemoryMode) -> SessionDir {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.owner = Some("did:bns:bob".into());
    spec.route_key = Some(format!("{AGENT}/{memory:?}"));
    spec.outbound = Some(OutboundBinding {
        to: "did:bns:bob".into(),
        to_session: Some(format!("{memory:?}")),
        kind: "chat".into(),
    });
    spec.policy.memory = memory;
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [{ "groupname": "bash_core" }] },
        "runtime": { "task_wait_ms": 8000 },
        "json_schema": {
            "type": "object", "required": ["reply", "topic", "artifacts"],
            "properties": {
                "reply": { "type": "string" },
                "topic": { "type": ["object", "null"] },
                "artifacts": { "type": "array" }
            }
        }
    });
    env.create_work(spec).await
}

/// One Turn in `sd`: the model records `note`, then replies with a topic.
/// Returns what its first inference saw.
async fn turn(env: &Env, sd: &SessionDir, says: &str, note: Option<&str>) -> String {
    post_blocking(&env.queue_dir, &queue_of(sd), msg(says));
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let (s2, note, queue) = (seen.clone(), note.map(str::to_string), env.queue_dir.clone());
    let llm = ScriptedLlm::new(move |req, n| {
        if n == 0 {
            *s2.lock().unwrap() = render(&req.messages);
            if let Some(note) = &note {
                return tool_call(
                    "c1",
                    "shell",
                    json!({ "command": format!("agent-session ctl perceive '{note}' --tag tea --queue-dir {}", queue.display()) }),
                );
            }
        }
        text(r#"{"reply":"ok","topic":{"title":"about tea","tags":["tea"]},"artifacts":[]}"#)
    });
    let mut deps = env
        .deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.options.load_hints = true;
    drive(sd, &deps, StopWhen::Idle).await;
    let seen = seen.lock().unwrap().clone();
    seen
}

async fn finds(env: &Env, sid: &str, words: &str) -> (bool, Option<String>) {
    let q = MemoryQuery {
        text: Some(words.to_string()),
        include_recent_perceptions: true,
        limit: 10,
        ..Default::default()
    };
    let r = env.agent().cognition().query(sid, &q).await.unwrap();
    (
        r.recent_perceptions.iter().any(|h| h.summary.contains(words)),
        r.note,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conversation_uses_memory_as_it_was_created() {
    let env = Env::new();
    let usual = chat(&env, MemoryMode::Normal).await;
    let quiet = chat(&env, MemoryMode::NoWrite).await;
    let blind = chat(&env, MemoryMode::NoReadWrite).await;
    turn(&env, &usual, "I like green tea", Some("the owner likes green tea")).await;

    // Reading: by itself and asked for.
    let seen = turn(&env, &quiet, "which green tea is best?", Some("the owner owns a samovar")).await;
    assert!(seen.contains("the owner likes green tea"), "{seen}");
    assert!(seen.contains("nothing of this conversation is remembered"), "{seen}");
    let seen = turn(&env, &blind, "which green tea is best?", Some("the owner owns a gaiwan")).await;
    assert!(!seen.contains("the owner likes green tea"), "{seen}");
    assert!(seen.contains("neither uses nor leaves memory"), "{seen}");
    assert_eq!(finds(&env, quiet.sid(), "green tea").await, (true, None));
    let (found, why) = finds(&env, blind.sid(), "green tea").await;
    assert!(!found && why.unwrap().contains("neither read nor write memory"));
    assert!(env.agent().cognition().read(blind.sid(), "p:x").await.unwrap_err().to_string().contains("memory is not available"));

    // Writing: nothing of either reaches anyone.
    for words in ["samovar", "gaiwan"] {
        assert!(!finds(&env, usual.sid(), words).await.0, "{words}");
    }
    let perceptions = std::fs::read_dir(env.agent_root.join("state").join("perception"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        !perceptions.iter().any(|f| f.contains(quiet.sid()) || f.contains(blind.sid())),
        "{perceptions:?}"
    );

    // Their topics stay theirs; the usual one's shows in other conversations.
    let topic = |sid: String| {
        let agent = env.agent();
        async move { agent.sessions().lookup(&sid).await.unwrap().unwrap().status.topic }
    };
    assert_eq!(topic(usual.sid().to_string()).await.unwrap().title, "about tea");
    for sd in [&quiet, &blind] {
        assert_eq!(sd.state().unwrap().topic.title, "about tea", "kept for itself");
        assert!(topic(sd.sid().to_string()).await.is_none());
    }

    // The work it starts, at every level.
    let agent = env.agent();
    for (sd, expected) in [(&usual, MemoryMode::Normal), (&quiet, MemoryMode::NoWrite), (&blind, MemoryMode::NoReadWrite)] {
        let mut parent = sd.sid().to_string();
        for level in 0..2 {
            let work = libopendan::api::create_sub_session(
                agent.as_ref(),
                APP,
                &parent,
                libopendan::api::SubSessionSpec {
                    objective: format!("work {level}"),
                    key: Some(format!("k{level}")),
                    ..Default::default()
                },
                env.channels().as_ref(),
            )
            .await
            .unwrap();
            assert_eq!(work.config().unwrap().session.policy.memory, expected);
            parent = work.sid().to_string();
        }
        let (found, why) = finds(&env, &parent, "green tea").await;
        assert_eq!(why.is_some(), expected == MemoryMode::NoReadWrite, "{found}");
    }
}
