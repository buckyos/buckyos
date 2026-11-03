//! Retries of a run that stopped on a retryable error (notepads/
//! opendan-dv-followup-todo.md H1): the run is kept and infers again only
//! after its backoff (`state.retry`, the registry's `retry_not_before_ms`);
//! retries in a row are bounded and then fail the run as
//! `provider_unavailable`; progress or any other outcome starts over; a user
//! message retries at once, other inputs wait; sub sessions in backoff are
//! not driven and take no slot; the backoff is not charged to the wallclock.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_tool::todo_tools::TodoStatus;
use buckyos_api::AiRole;
use common::*;
use libopendan::api::{create_sub_session, SubSessionSpec};
use libopendan::host::{ChildDriver, HostDeps};
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, RunnerDeps, StopWhen, PROVIDER_UNAVAILABLE};
use libopendan::state::AgentStateClient;
use libopendan::SessionDir;
use llm_context::deps::LlmInferenceRequest;
use llm_context::error::{LLMComputeError, ProviderFailure};
use serde_json::json;

fn busy() -> LLMComputeError {
    LLMComputeError::Provider {
        failure: ProviderFailure::Transient,
        message: "503 upstream overloaded".into(),
    }
}

/// Runner deps with the given backoff (`base`, cap 4 × base) and limit.
fn deps(env: &Env, llm: Arc<ScriptedLlm>, base_ms: u64, max_attempts: u32) -> RunnerDeps {
    let mut d = env.deps(llm);
    d.options.retry_backoff_base = Duration::from_millis(base_ms);
    d.options.retry_backoff_cap = Duration::from_millis(base_ms * 4);
    d.options.max_retry_attempts = max_attempts;
    d.options.max_retry_duration = Duration::from_secs(600);
    d
}

fn retry_of(sd: &SessionDir) -> Option<RetryState> {
    sd.state().unwrap().retry
}

async fn status_of(env: &Env, sd: &SessionDir) -> SessionStatus {
    env.agent()
        .sessions()
        .lookup(sd.sid())
        .await
        .unwrap()
        .unwrap()
        .status
}

/// Wait until the retry of `sd` is due.
async fn until_due(sd: &SessionDir) {
    if let Some(r) = retry_of(sd) {
        let now = libopendan::now_ms();
        if r.not_before_ms > now {
            tokio::time::sleep(Duration::from_millis(r.not_before_ms - now + 20)).await;
        }
    }
}

fn assert_recoverable(r: &DriveResult) {
    match r {
        DriveResult::Error { error, .. } => {
            assert_eq!(error["recoverable"], json!(true), "{error}")
        }
        r => panic!("{r:?}"),
    }
}

/// Fails the first `n` inferences, then answers.
fn failing_first(n: usize) -> Arc<ScriptedLlm> {
    ScriptedLlm::fallible(move |_, i| {
        if i < n {
            Err(busy())
        } else {
            Ok(text("all done"))
        }
    })
}

#[tokio::test]
async fn a_retryable_error_waits_for_its_backoff_and_the_same_run_goes_on() {
    let env = Env::new();
    let sd = env
        .create_work(work_spec("answer once the provider is back"))
        .await;
    let llm = failing_first(2);
    let d = deps(&env, llm.clone(), 400, 6);
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert_recoverable(&r);
    let first = retry_of(&sd).expect("a retry is scheduled");
    assert_eq!(first.attempts, 1);
    assert!(first.not_before_ms >= first.since_ms + 400, "{first:?}");
    assert_eq!(
        status_of(&env, &sd).await.retry_not_before_ms,
        Some(first.not_before_ms)
    );
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    // In its backoff the run does not infer, however often it is driven.
    for until in [StopWhen::Idle, StopWhen::Finished, StopWhen::TurnClosed] {
        let r = drive(&sd, &d, until).await;
        assert_recoverable(&r);
        if let DriveResult::Error { error, .. } = &r {
            assert_eq!(error["retry_not_before_ms"], json!(first.not_before_ms));
        }
    }
    assert_eq!(llm.count(), 1);
    assert_eq!(retry_of(&sd), Some(first.clone()));
    // Due: retried, fails again — the next backoff doubles.
    until_due(&sd).await;
    assert_recoverable(&drive(&sd, &d, StopWhen::Finished).await);
    assert_eq!(llm.count(), 2);
    let second = retry_of(&sd).unwrap();
    assert_eq!((second.attempts, second.since_ms), (2, first.since_ms));
    assert!(
        second.not_before_ms >= first.not_before_ms + 800,
        "{second:?}"
    );
    // Due again: the same run answers; nothing of the retries is left.
    until_due(&sd).await;
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 3);
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!(
        st.last_run.as_deref(),
        Some(run_id.as_str()),
        "same run continued"
    );
    assert!(st.retry.is_none() && st.last_error.is_none(), "{st:?}");
    assert_eq!(status_of(&env, &sd).await.retry_not_before_ms, None);
    assert_eq!(st.turn_seq, 1, "one Turn, kept open across the retries");
}

/// R2: a retryable error after the segment made progress (successful
/// inferences, tool calls) starts the retries over; only errors in a row
/// count.
#[tokio::test]
async fn progress_between_retryable_errors_starts_the_retries_over() {
    let env = Env::new();
    let sd = env
        .create_work(work_spec("a long run with a flaky provider"))
        .await;
    let llm = ScriptedLlm::fallible(|req: &LlmInferenceRequest, i| match i {
        0 | 2 | 4 => Err(busy()),
        _ if has_tool_result(req, "c2").is_some() => Ok(text("all done")),
        _ if has_tool_result(req, "c1").is_some() => Ok(tool_call(
            "c2",
            "shell",
            json!({ "command": "echo two >> steps.log" }),
        )),
        _ => Ok(tool_call(
            "c1",
            "shell",
            json!({ "command": "echo one >> steps.log" }),
        )),
    });
    // At most one retry in a row: a counted second error would fail it.
    let d = deps(&env, llm.clone(), 100, 1);
    assert_recoverable(&drive(&sd, &d, StopWhen::Finished).await);
    let first = retry_of(&sd).unwrap();
    for _ in 0..2 {
        until_due(&sd).await;
        // Retried: one tool call succeeds, the next inference fails again.
        assert_recoverable(&drive(&sd, &d, StopWhen::Finished).await);
        let r = retry_of(&sd).unwrap();
        assert_eq!(r.attempts, 1, "{r:?}");
        assert!(r.since_ms > first.since_ms, "a new series: {r:?}");
    }
    until_due(&sd).await;
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 6);
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
}

/// R5: errors in a row past the limit fail the run as
/// `provider_unavailable`; the count is kept in state (a runner that
/// restarts goes on counting), and a Todo's observation of the failed
/// session says why.
#[tokio::test]
async fn retries_used_up_fail_the_session_as_provider_unavailable() {
    let env = Env::new();
    let mut spec = work_spec("nobody answers this one");
    spec.policy.wait_user_msg = WaitPolicy::FinishFailed;
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::fallible(|_, _| Err(busy()));
    let mut since = None;
    for n in 1..=2 {
        // A new runner every time: the count comes from state.json.
        let r = drive(&sd, &deps(&env, llm.clone(), 100, 2), StopWhen::Finished).await;
        assert_recoverable(&r);
        let retry = retry_of(&sd).unwrap();
        assert_eq!(retry.attempts, n);
        assert_eq!(*since.get_or_insert(retry.since_ms), retry.since_ms);
        until_due(&sd).await;
    }
    let r = drive(&sd, &deps(&env, llm.clone(), 100, 2), StopWhen::Finished).await;
    assert!(
        matches!(
            &r,
            DriveResult::Finished {
                outcome: Some(Outcome::Failed),
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(llm.count(), 3, "two retries after the first error");
    let st = sd.state().unwrap();
    let e = st.last_error.clone().unwrap();
    assert_eq!(e["kind"], PROVIDER_UNAVAILABLE);
    assert_eq!(e["retries_exhausted"], true);
    assert_eq!(e["attempts"], 2);
    assert_eq!(e["since_ms"], json!(since.unwrap()));
    assert_eq!(e["recoverable"], false);
    assert!(
        e["message"]
            .as_str()
            .unwrap()
            .contains("503 upstream overloaded"),
        "{e}"
    );
    assert!(st.retry.is_none());
    let ended: Vec<TurnStatus> = read_worklog(&sd)
        .into_iter()
        .filter_map(|w| match w.body {
            WorklogBody::TurnEnded { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(ended, vec![TurnStatus::Failed]);
    // What a plan sees of this Todo's work session.
    let entry = env
        .agent()
        .sessions()
        .lookup(sd.sid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.status.retry_not_before_ms, None);
    let seen = libopendan::todos::observe(&entry, Some(&st));
    assert_eq!(seen.status, TodoStatus::Failed);
    let why = seen.error.unwrap();
    assert!(why.starts_with(PROVIDER_UNAVAILABLE), "{why}");
    assert!(why.contains("gave up after 2 retries"), "{why}");
    assert!(why.contains("503 upstream overloaded"), "{why}");
    assert!(seen.summary.unwrap().contains("gave up after 2 retries"));
}

/// Q3 = A: a user message among the inputs retries at once (and joins the
/// batch); other inputs — a timer, a task's end — wait for the backoff.
#[tokio::test]
async fn a_user_message_retries_at_once_other_inputs_wait() {
    let env = Env::new();
    let mut spec = work_spec("answer the user");
    spec.subscriptions.push(Subscription {
        id: "t".into(),
        mode: SubscriptionMode::Active,
        source: SubscriptionSource::Timer { name: "t1".into() },
        watch: vec![],
    });
    let sd = env.create_work(spec).await;
    let llm = failing_first(1);
    // The production backoff: nothing in this test waits it out.
    let d = deps(&env, llm.clone(), 30_000, 6);
    assert_recoverable(&drive(&sd, &d, StopWhen::Finished).await);
    let applied = |sd: &SessionDir| sd.state().unwrap().live_run.unwrap().applied_input_seq;
    let batches = applied(&sd);
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &event("tick:1", Some("t"), "timer", "t1", None, "timer fired"),
    )
    .await
    .unwrap();
    assert_recoverable(&drive(&sd, &d, StopWhen::Idle).await);
    assert_eq!(llm.count(), 1, "a timer does not cut the backoff short");
    assert_eq!(applied(&sd), batches, "it is not taken into the run");
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("are you there?"))
        .await
        .unwrap();
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    let t = llm.transcript(1);
    assert!(
        t.contains("are you there?") && t.contains("timer fired"),
        "{t}"
    );
    let st = sd.state().unwrap();
    assert!(st.retry.is_none() && st.last_error.is_none(), "{st:?}");
}

/// R3: the wallclock budget measures a stretch of running; a backoff
/// longer than it does not end the retried run in a wrap-up.
#[tokio::test]
async fn the_backoff_is_not_charged_to_the_wallclock() {
    let env = Env::new();
    let mut spec = work_spec("retry after a long pause");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true }, "timeout": 1 });
    let sd = env.create_work(spec).await;
    let llm = failing_first(1);
    let d = deps(&env, llm.clone(), 1_500, 6);
    assert_recoverable(&drive(&sd, &d, StopWhen::Finished).await);
    until_due(&sd).await;
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert!(
        matches!(
            &r,
            DriveResult::Finished {
                outcome: Some(Outcome::Succeeded),
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(llm.count(), 2);
    let wrap_ups = read_worklog(&sd)
        .into_iter()
        .filter(|w| matches!(&w.body, WorklogBody::ControlApplied { command, .. } if command == "budget_wrap_up"))
        .count();
    assert_eq!(wrap_ups, 0);
    assert!(!llm.transcript(1).contains("<<budget>>"));
}

fn child_host(env: &Env, llm: Arc<ScriptedLlm>, base_ms: u64, slots: usize) -> HostDeps {
    let d = deps(env, llm, base_ms, 6);
    HostDeps {
        who: APP.into(),
        agent: d.agent.clone(),
        inputs: d.inputs.clone(),
        waker: d.waker.clone(),
        xllm: d.xllm.clone(),
        assembler: Arc::new(libopendan::runner::DefaultAssembler::default()),
        session_cli: None,
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        context_windows: None,
        options: d.options.clone(),
        max_child_concurrency: slots,
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

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A child that is always busy, one that answers; inferences of the busy one.
fn children_llm(busy_calls: Arc<AtomicUsize>) -> Arc<ScriptedLlm> {
    ScriptedLlm::fallible(move |req, _| {
        if system_of(req).contains("busy child work") {
            busy_calls.fetch_add(1, Ordering::SeqCst);
            Err(busy())
        } else {
            Ok(tool_call(
                "done",
                "report",
                json!({ "report": "quick result", "is_end": true }),
            ))
        }
    })
}

async fn sub(env: &Env, parent: &SessionDir, objective: &str, key: &str) -> SessionDir {
    create_sub_session(
        env.agent().as_ref(),
        APP,
        parent.sid(),
        SubSessionSpec {
            objective: objective.into(),
            key: Some(key.into()),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap()
}

/// Ticks the sub session driver for `d`.
async fn tick_for(children: &ChildDriver, roots: &[String], d: Duration) {
    let started = Instant::now();
    while started.elapsed() < d {
        children.tick(roots).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// R1 / R4: the sub session driver ticks every ≤ 1 s, but a sub session
/// in backoff is driven again only when its retry is due — and meanwhile
/// it takes no slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sub_session_in_backoff_is_not_driven_and_takes_no_slot() {
    let env = Env::new();
    let parent = env.create_work(work_spec("delegate")).await;
    // Created second: looked at first by the driver.
    let quick = sub(&env, &parent, "quick child work", "quick").await;
    let busy_child = sub(&env, &parent, "busy child work", "busy").await;
    assert!(busy_child.sid() > quick.sid());
    let calls = Arc::new(AtomicUsize::new(0));
    let children = ChildDriver::new(child_host(&env, children_llm(calls.clone()), 10_000, 1));
    let roots = vec![parent.sid().to_string()];
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10)
        && (started.elapsed() < Duration::from_millis(1_500)
            || !quick.state().unwrap().is_finished())
    {
        tick_for(&children, &roots, Duration::from_millis(100)).await;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "driven once, then left alone"
    );
    assert!(retry_of(&busy_child).is_some());
    let st = quick.state().unwrap();
    assert_eq!(
        st.outcome,
        Some(Outcome::Succeeded),
        "the one slot went to the other child"
    );
    children.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sub_session_is_driven_again_when_its_retry_is_due() {
    let env = Env::new();
    let parent = env.create_work(work_spec("delegate")).await;
    let busy_child = sub(&env, &parent, "busy child work", "busy").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let children = ChildDriver::new(child_host(&env, children_llm(calls.clone()), 600, 4));
    let roots = vec![parent.sid().to_string()];
    // Retries at ~0.6 s (then ~1.2 s later): two inferences in 1.4 s, not
    // one per tick.
    tick_for(&children, &roots, Duration::from_millis(1_400)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(retry_of(&busy_child).unwrap().attempts, 2);
    children.shutdown().await;
}
