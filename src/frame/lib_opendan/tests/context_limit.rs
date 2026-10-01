//! X7: a run that hits the context limit is compacted mid-run — its history
//! goes to the worklog first, the session history is compacted, the run
//! continues from system + history in a new epoch — and every record stays
//! in the worklog exactly once, in its Turn.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::AiRole;
use common::*;
use libopendan::protocol::*;
use libopendan::runner::Summarizer;
use libopendan::runner::{drive, StopWhen};
use llm_context::error::{LLMComputeError, ProviderFailure};
use serde_json::json;

struct StaticSummarizer;

#[async_trait]
impl Summarizer for StaticSummarizer {
    async fn summarize(
        &self,
        _previous: &str,
        _segment: &str,
    ) -> libopendan::error::Result<String> {
        Ok("SUMMARY-X".into())
    }
}

fn refusal() -> LLMComputeError {
    LLMComputeError::provider(ProviderFailure::ContextLimit, "context_length_exceeded")
}

fn results_of(sd: &libopendan::SessionDir) -> Vec<(String, u64)> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, turn, .. } => Some((call_id, turn)),
            _ => None,
        })
        .collect()
}

fn rewrites(sd: &libopendan::SessionDir) -> usize {
    read_worklog(sd)
        .iter()
        .filter(
            |e| matches!(&e.body, WorklogBody::Outcome { kind, .. } if kind == "context_rewritten"),
        )
        .count()
}

#[tokio::test]
async fn function_call_run_continues_after_a_mid_run_rewrite() {
    let env = Env::new();
    let sd = env.create_work(work_spec("two tool steps")).await;
    let llm = ScriptedLlm::fallible(|req, _| {
        let all = render(&req.messages);
        if has_tool_result(req, "c2").is_some() {
            return Ok(text("all done"));
        }
        if has_tool_result(req, "c1").is_some() {
            // The raw transcript no longer fits the model.
            return Err(refusal());
        }
        if all.contains("<session_history>") && all.contains("[result #c1") {
            return Ok(tool_call("c2", "exec", json!({ "command": "echo two" })));
        }
        Ok(tool_call("c1", "exec", json!({ "command": "echo one" })))
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4);

    // After the rewrite the request is system + session history, carrying
    // the Turn's input and the first tool call.
    let reqs = llm.requests.lock().unwrap().clone();
    let rewritten = &reqs[2].messages;
    assert_eq!(rewritten.len(), 2, "{}", render(rewritten));
    assert_eq!(rewritten[0].role, AiRole::System);
    let history = rewritten[1].text_content();
    assert!(
        history.contains("[input]") && history.contains("[result #c1 ok]"),
        "{history}"
    );

    // Every record once, all in Turn 1, in order; the Turn ends once.
    assert_eq!(
        results_of(&sd),
        vec![("c1".to_string(), 1), ("c2".to_string(), 1)]
    );
    assert_eq!(rewrites(&sd), 1);
    let k = kinds(&read_worklog(&sd));
    assert_eq!(
        k,
        vec![
            "created",
            "turn_started",
            "user_message",
            "assistant_message",
            "action_result",
            "outcome",
            "assistant_message",
            "action_result",
            "assistant_message",
            "outcome",
            "turn_ended"
        ]
    );
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert!(st.open_turn.is_none());

    // The same run went on in epoch 1.
    let st = sd.state().unwrap();
    let run_id = st.last_run.clone().unwrap();
    assert_eq!(sd.runs().list().unwrap(), vec![run_id.clone()]);
    let (_, snap) = sd.runs().load_checked(&run_id).unwrap();
    let snap = snap.unwrap();
    let meta: HostMeta =
        serde_json::from_value(snap.state.host.as_ref().unwrap()[HOST_META_KEY].clone()).unwrap();
    assert_eq!(meta.history_epoch, 1);
    assert_eq!(meta.epoch_turn, 1);
    assert_eq!(meta.epoch_input_seq, 1);
    assert_eq!(snap.request.input.len(), 2);
    assert!(snap.state.suspended.is_none());
}

#[tokio::test]
async fn behavior_run_compacts_the_session_history_and_keeps_step_numbering() {
    let env = Env::new();
    let mut spec = work_spec("behavior with a tight history budget");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    spec.prompt.history_budget_tokens = Some(80);
    let sd = env.create_work(spec).await;
    let refused = Arc::new(AtomicBool::new(false));
    let seen_refusal = refused.clone();
    let llm = ScriptedLlm::fallible(move |req, _| {
        let all = render(&req.messages);
        if seen_refusal.load(Ordering::SeqCst) {
            return Ok(text(
                "<response><report><![CDATA[finished after compaction]]></report></response>",
            ));
        }
        if all.contains("behavior-1") && all.contains("last_step_action_results") {
            seen_refusal.store(true, Ordering::SeqCst);
            return Err(refusal());
        }
        Ok(text(
            "<response><thinking>go</thinking><actions><exec><![CDATA[echo behavior-1]]></exec></actions></response>",
        ))
    });
    let deps = env
        .deps(llm.clone())
        .with_summarizer(Arc::new(StaticSummarizer));
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(refused.load(Ordering::SeqCst));
    assert_eq!(llm.count(), 3);

    // The session history was compacted (summary.json) and the rewritten
    // prompt starts from it.
    let reqs = llm.requests.lock().unwrap().clone();
    let after = render(&reqs[2].messages);
    assert!(after.contains("SUMMARY-X"), "{after}");
    assert!(
        !after.contains("last_step_action_results"),
        "folded into the history: {after}"
    );
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(|e| e.body.kind() == "compaction"));
    assert_eq!(rewrites(&sd), 1);
    let steps = wl.iter().filter(|e| e.body.kind() == "step").count();
    assert_eq!(steps, 2, "each step once");
    assert_eq!(results_of(&sd).len(), 1);

    let st = sd.state().unwrap();
    let (_, snap) = sd
        .runs()
        .load_checked(st.last_run.as_ref().unwrap())
        .unwrap();
    let snap = snap.unwrap();
    assert_eq!(snap.state.next_step_index, 2, "numbering continues");
    let indices: Vec<u32> = snap.state.steps.iter().map(|s| s.meta.step_index).collect();
    assert_eq!(indices, vec![1]);
    assert!(sd.report().unwrap().contains("finished after compaction"));
}

#[tokio::test]
async fn a_run_that_never_fits_is_paused_with_the_limit_and_retried_later() {
    let env = Env::new();
    let sd = env.create_work(work_spec("never fits")).await;
    let fits = Arc::new(AtomicBool::new(false));
    let now_fits = fits.clone();
    let llm = ScriptedLlm::fallible(move |_, _| {
        if now_fits.load(Ordering::SeqCst) {
            Ok(text("finally"))
        } else {
            Err(refusal())
        }
    });
    let deps = env.deps(llm.clone());
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(
        matches!(&r, libopendan::runner::DriveResult::Error { error, .. } if error["kind"] == "context_limit"),
        "{r:?}"
    );
    // One try plus three compactions, then the run is paused at the limit.
    // Only the first rewrite closed new history.
    assert_eq!(llm.count(), 4);
    assert_eq!(rewrites(&sd), 1);
    let st = sd.state().unwrap();
    let live = st.live_run.clone().expect("the run is kept");
    // The pause is resumable: the Turn stays open.
    assert_eq!(st.open_turn.as_ref().map(|t| t.index), Some(1));
    assert_eq!(st.turns_completed, 0);
    // Four Rounds, all refused by the provider.
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.rounds_failed), (4, 4));
    let (rec, snap) = sd.runs().load_checked(&live.run_id).unwrap();
    assert_eq!(rec.usage.llm_requests, 4);
    assert_eq!(rec.status, agent_tool::local_llm_context::RunStatus::Paused);
    assert!(matches!(
        snap.unwrap().state.suspended,
        Some(llm_context::Suspension::ContextLimit { .. })
    ));

    // A later drive compacts the suspended run and continues it.
    fits.store(true, Ordering::SeqCst);
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(
        sd.state().unwrap().last_run.as_deref(),
        Some(live.run_id.as_str())
    );
    assert_eq!(rewrites(&sd), 1);
    let users = read_worklog(&sd)
        .iter()
        .filter(|e| e.body.kind() == "user_message")
        .count();
    assert_eq!(users, 1, "the Turn input is written once");
    // The context limit, the pause and the later drive are one Turn.
    let wl = read_worklog(&sd);
    let k = kinds(&wl);
    assert_eq!(k.iter().filter(|k| **k == "turn_started").count(), 1);
    assert_eq!(k.iter().filter(|k| **k == "turn_ended").count(), 1);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let stats = sd.statistics().unwrap();
    assert_eq!((stats.rounds, stats.rounds_failed, stats.turns), (5, 4, 1));
}
