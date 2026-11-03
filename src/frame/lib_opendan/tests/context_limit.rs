//! X7: a run that hits the context limit is compacted mid-run — its history
//! goes to the worklog first, the session history is compacted, the run
//! continues from system + history in a new epoch — and every record stays
//! in the worklog exactly once, in its Turn.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiResponse, AiRole};
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::{LLMComputeError, ProviderFailure};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
struct ContextLogEvent {
    name: String,
    level: log::Level,
    body: Value,
}

struct ContextTestLogger(Mutex<Vec<ContextLogEvent>>);

static CONTEXT_TEST_LOGGER: ContextTestLogger = ContextTestLogger(Mutex::new(Vec::new()));
static INIT_CONTEXT_TEST_LOGGER: Once = Once::new();

impl log::Log for ContextTestLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        let message = record.args().to_string();
        let Some((name, body)) = message.split_once(' ') else {
            return;
        };
        if name != "llm_context.context_limit" && !name.starts_with("opendan.context.") {
            return;
        }
        let Ok(body) = serde_json::from_str(body) else {
            return;
        };
        self.0.lock().unwrap().push(ContextLogEvent {
            name: name.to_string(),
            level: record.level(),
            body,
        });
    }

    fn flush(&self) {}
}

/// Tests of this file share the process logger and, with a fresh root
/// each, the same session ids: one at a time, each with its own capture.
static CONTEXT_TEST_SERIAL: Mutex<()> = Mutex::new(());

fn init_context_logger() -> std::sync::MutexGuard<'static, ()> {
    let guard = CONTEXT_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    INIT_CONTEXT_TEST_LOGGER.call_once(|| {
        log::set_logger(&CONTEXT_TEST_LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
    CONTEXT_TEST_LOGGER.0.lock().unwrap().clear();
    guard
}

fn context_log_events(session_id: &str) -> Vec<ContextLogEvent> {
    CONTEXT_TEST_LOGGER
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| {
            event.body["session_id"] == session_id
                || event.body["owner"]["session_id"] == session_id
        })
        .cloned()
        .collect()
}

fn events_named<'a>(events: &'a [ContextLogEvent], name: &str) -> Vec<&'a ContextLogEvent> {
    events.iter().filter(|event| event.name == name).collect()
}

/// The summary model: one fixed summary, every prompt kept.
struct StaticSummarizer {
    summary: String,
    prompts: Mutex<Vec<String>>,
}

impl StaticSummarizer {
    fn new(summary: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            summary: summary.into(),
            prompts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl LlmClient for StaticSummarizer {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.prompts.lock().unwrap().push(render(&req.messages));
        Ok(text(&json!({ "summary": self.summary }).to_string()))
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
    let _serial = init_context_logger();
    let env = Env::new();
    let sd = env.create_work(work_spec("two tool steps")).await;
    let llm = ScriptedLlm::fallible(|req, _| {
        if has_tool_result(req, "c2").is_some() {
            return Ok(text("all done"));
        }
        if render(&req.messages).contains("<history_summary>") {
            return Ok(tool_call("c2", "shell", json!({ "command": "echo two" })));
        }
        if has_tool_result(req, "c1").is_some() {
            // The raw transcript no longer fits the model.
            return Err(refusal());
        }
        Ok(tool_call("c1", "shell", json!({ "command": "echo one" })))
    });
    let summarizer = StaticSummarizer::new("SUMMARY-X");
    let deps = env.deps(llm.clone()).with_summarizer(summarizer.clone());
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4);

    // The provider refused the transcript: the rewrite brings it under half
    // of that, here system + the summary block. The summarizer read the
    // history rebuilt from the worklog: the Turn's input, the first tool
    // call and its result.
    let reqs = llm.requests.lock().unwrap().clone();
    let rewritten = &reqs[2].messages;
    assert_eq!(rewritten.len(), 2, "{}", render(rewritten));
    assert_eq!(rewritten[0].role, AiRole::System);
    assert!(rewritten[1].text_content().starts_with("<history_summary>\nSUMMARY-X"));
    let prompt = summarizer.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("call shell #c1") && prompt.contains("result #c1 [ok]"),
        "{prompt}"
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
            "compaction",
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

    let events = context_log_events(sd.sid());
    let limits = events_named(&events, "llm_context.context_limit");
    assert_eq!(limits.len(), 1, "{events:?}");
    assert_eq!(limits[0].level, log::Level::Info);
    assert_eq!(limits[0].body["reason"], "provider_refused");
    assert_eq!(limits[0].body["trace_id"], run_id);
    let started = events_named(&events, "opendan.context.rewrite.started");
    let completed = events_named(&events, "opendan.context.rewrite.completed");
    assert_eq!((started.len(), completed.len()), (1, 1), "{events:?}");
    assert_eq!(started[0].body["reason"], "provider_refused");
    assert_eq!(started[0].body["attempt"], 1);
    for event in [started[0], completed[0]] {
        assert_eq!(event.body["run_id"], run_id);
        assert_eq!(event.level, log::Level::Info);
    }
    assert_eq!(completed[0].body["summarized"], true);
    assert!(completed[0].body["history_tokens_after"].as_u64().unwrap() > 0);
    assert!(events_named(&events, "opendan.context.rewrite.failed").is_empty());
    assert!(events_named(&events, "opendan.context.rewrite.paused").is_empty());
}

#[tokio::test]
async fn behavior_run_compacts_the_session_history_and_keeps_step_numbering() {
    let _serial = init_context_logger();
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
                "<response><report end=\"true\"><![CDATA[finished after compaction]]></report></response>",
            ));
        }
        if all.contains("behavior-1") && all.contains("last_step_action_results") {
            seen_refusal.store(true, Ordering::SeqCst);
            return Err(refusal());
        }
        Ok(text(
            "<response><intent>go</intent><actions><shell><![CDATA[echo behavior-1]]></shell></actions></response>",
        ))
    });
    // No window: the history budget is the rewrite's whole target.
    let mut deps = env
        .deps(llm.clone())
        .with_summarizer(StaticSummarizer::new("SUMMARY-X"));
    deps.options.default_context_window = None;
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

    let events = context_log_events(sd.sid());
    let triggered = events_named(&events, "opendan.context.compaction.triggered");
    assert!(
        triggered
            .iter()
            .any(|event| event.body["reason"] == "context_limit"),
        "{events:?}"
    );
    let completed = events_named(&events, "opendan.context.compaction.completed");
    assert!(!completed.is_empty(), "{events:?}");
    assert!(completed
        .iter()
        .any(|event| event.body["reason"] == "context_limit"));
    assert!(events_named(&events, "opendan.context.compaction.failed").is_empty());
    let rewritten = events_named(&events, "opendan.context.rewrite.completed");
    assert_eq!(rewritten.len(), 1, "{events:?}");
    assert_eq!(rewritten[0].body["run_id"], st.last_run.as_deref().unwrap());
    assert_eq!(rewritten[0].body["summarized"], true);
}

#[tokio::test]
async fn a_run_that_never_fits_is_paused_with_the_limit_and_retried_later() {
    let _serial = init_context_logger();
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
    let deps = env
        .deps(llm.clone())
        .with_summarizer(StaticSummarizer::new("SUMMARY-X"));
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
    assert_eq!(rec.status, agent_tool::xllm::RunStatus::Paused);
    assert!(matches!(
        snap.unwrap().state.suspended,
        Some(llm_context::Suspension { .. })
    ));

    let events = context_log_events(sd.sid());
    assert_eq!(events_named(&events, "llm_context.context_limit").len(), 4);
    let started = events_named(&events, "opendan.context.rewrite.started");
    assert_eq!(started.len(), 3, "{events:?}");
    for (index, event) in started.iter().enumerate() {
        assert_eq!(event.body["attempt"], index + 1);
        assert_eq!(event.body["run_id"], live.run_id);
        assert_eq!(event.body["reason"], "provider_refused");
    }
    assert_eq!(
        events_named(&events, "opendan.context.rewrite.completed").len(),
        3
    );
    let paused = events_named(&events, "opendan.context.rewrite.paused");
    assert_eq!(paused.len(), 1, "{events:?}");
    assert_eq!(paused[0].level, log::Level::Warn);
    assert_eq!(paused[0].body["run_id"], live.run_id);
    assert_eq!(paused[0].body["attempt"], 3);
    assert_eq!(paused[0].body["max_attempts"], 3);

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

struct FailingSummarizer;

#[async_trait]
impl LlmClient for FailingSummarizer {
    async fn infer(&self, _req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        Err(LLMComputeError::provider(
            ProviderFailure::Permanent,
            "summary backend unavailable",
        ))
    }
}

#[tokio::test]
async fn failed_compaction_logs_the_failure_and_preserves_the_error() {
    let _serial = init_context_logger();
    let env = Env::new();
    let mut spec = work_spec("summary failure");
    spec.prompt.history_budget_tokens = Some(80);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::fallible(|req, _| {
        if has_tool_result(req, "large-result").is_some() {
            return Err(refusal());
        }
        Ok(tool_call(
            "large-result",
            "shell",
            json!({"command": format!("echo {}", "x".repeat(1024))}),
        ))
    });
    let mut deps = env.deps(llm).with_summarizer(Arc::new(FailingSummarizer));
    deps.options.default_context_window = None;
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    let libopendan::runner::DriveResult::Error { error, .. } = result else {
        panic!("expected summarizer failure, got {result:?}");
    };
    assert_eq!(error["kind"], "llm");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("summary backend unavailable"),
        "{error}"
    );
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let events = context_log_events(sd.sid());
    for phase in ["compaction", "rewrite"] {
        let started = events_named(&events, &format!("opendan.context.{phase}.started"));
        let failed = events_named(&events, &format!("opendan.context.{phase}.failed"));
        assert_eq!((started.len(), failed.len()), (1, 1), "{events:?}");
        assert_eq!(failed[0].level, log::Level::Warn);
        assert!(failed[0]
            .body
            .to_string()
            .contains("summary backend unavailable"));
        assert!(events_named(&events, &format!("opendan.context.{phase}.completed")).is_empty());
    }
    let failed = events_named(&events, "opendan.context.rewrite.failed");
    assert_eq!(failed[0].body["run_id"], run_id);
}

// ---------------------------------------------------------------------
// The window and the target line (work context budget TODO §3.1)
// ---------------------------------------------------------------------

use libopendan::runner::history::LimitLine;
use libopendan::runner::{CachedContextWindows, ContextWindowSource, RunnerOptions};
use llm_context::context_window::estimate_request;
use llm_context::deps::ByteHeuristicTokenizer;

#[test]
fn the_rewrite_target_is_a_share_of_the_window() {
    // 128k: 30% is 38,400 tokens; the system prefix takes its part.
    let line = LimitLine::new(Some(128_000), 0.30, 2_000, 24_000);
    assert_eq!(line.target_tokens, Some(38_400));
    assert_eq!(line.history_budget, 36_400);
    // What the prefix leaves is less than the history budget: the budget
    // is the floor.
    let line = LimitLine::new(Some(128_000), 0.30, 20_000, 24_000);
    assert_eq!(line.history_budget, 24_000);
    // 400k: no longer squeezed to 6%.
    let line = LimitLine::new(Some(400_000), 0.30, 4_000, 24_000);
    assert_eq!((line.target_tokens, line.history_budget), (Some(120_000), 116_000));
    // An unknown window: the history budget as it was.
    let line = LimitLine::new(None, 0.30, 2_000, 24_000);
    assert_eq!((line.target_tokens, line.history_budget), (None, 24_000));
}

/// A window source as the host's AICC lookup: one fixed answer, every
/// alias it was asked for.
struct Windows {
    answer: Result<Option<u32>, String>,
    asked: Mutex<Vec<String>>,
}

impl Windows {
    fn new(answer: Result<Option<u32>, String>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ContextWindowSource for Windows {
    async fn context_window(&self, model: &str) -> Result<Option<u32>, String> {
        self.asked.lock().unwrap().push(model.to_string());
        self.answer.clone()
    }
}

/// The window a one-reply run was given: run.json limits and the request.
async fn run_window(
    llm_context: Value,
    source: Option<Arc<dyn ContextWindowSource>>,
    default: Option<u32>,
) -> (Option<u32>, Option<llm_context::ContextThreshold>) {
    run_window_capped(llm_context, source, default, None).await
}

/// [`run_window`] under `RunnerOptions.max_context_window = max`.
async fn run_window_capped(
    llm_context: Value,
    source: Option<Arc<dyn ContextWindowSource>>,
    default: Option<u32>,
    max: Option<u32>,
) -> (Option<u32>, Option<llm_context::ContextThreshold>) {
    let env = Env::new();
    let mut spec = work_spec("which window");
    spec.prompt.llm_context = llm_context;
    let sd = env.create_work(spec).await;
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("done")));
    deps.context_windows = source;
    deps.options.default_context_window = default;
    deps.options.max_context_window = max;
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let run_id = sd.state().unwrap().last_run.unwrap();
    let (record, snap) = sd.runs().load_checked(&run_id).unwrap();
    let snap = snap.unwrap();
    assert_eq!(
        record.config.limits.context_window_tokens,
        snap.request.budget.context_window_tokens
    );
    (
        snap.request.budget.context_window_tokens,
        snap.request.budget.context_yield_threshold,
    )
}

#[tokio::test]
async fn a_run_gets_the_window_of_its_model() {
    let tools = json!({ "tools": { "enabled": true } });
    let ratio = Some(llm_context::ContextThreshold::Ratio { value: 0.8 });

    // The model's window, asked by the run's alias (xllm's default one).
    let windows = Windows::new(Ok(Some(200_000)));
    let got = run_window(tools.clone(), Some(windows.clone()), Some(128_000)).await;
    assert_eq!(got, (Some(200_000), ratio.clone()));
    assert_eq!(*windows.asked.lock().unwrap(), vec!["llm.chat".to_string()]);
    let windows = Windows::new(Ok(Some(400_000)));
    let got = run_window(
        json!({ "model": "llm.code", "tools": { "enabled": true } }),
        Some(windows.clone()),
        Some(128_000),
    )
    .await;
    assert_eq!(got.0, Some(400_000));
    assert_eq!(*windows.asked.lock().unwrap(), vec!["llm.code".to_string()]);

    // An explicit `context_window` wins; nothing is looked up.
    let windows = Windows::new(Ok(Some(200_000)));
    let got = run_window(
        json!({ "context_window": 50_000, "tools": { "enabled": true } }),
        Some(windows.clone()),
        Some(128_000),
    )
    .await;
    assert_eq!(got, (Some(50_000), ratio.clone()));
    assert!(windows.asked.lock().unwrap().is_empty());

    // A failed lookup or a model without a window: the default; the run
    // is not held up.
    for answer in [Err("aicc unreachable".to_string()), Ok(None)] {
        let got = run_window(tools.clone(), Some(Windows::new(answer)), Some(128_000)).await;
        assert_eq!(got, (Some(128_000), ratio.clone()));
    }
    // No source: the default; no default either: no window, no early
    // yield (only the provider's refusal rewrites).
    assert_eq!(run_window(tools.clone(), None, Some(128_000)).await.0, Some(128_000));
    assert_eq!(run_window(tools.clone(), None, None).await, (None, None));

    // No upper bound by default (compaction TODO W1): the model's window.
    assert_eq!(RunnerOptions::default().max_context_window, None);
    let got = run_window(tools.clone(), Some(Windows::new(Ok(Some(1_050_000)))), Some(128_000))
        .await;
    assert_eq!(got.0, Some(1_050_000));
    // A bound set by the host: a larger model window is bounded, a smaller
    // one and the default stay, an explicit `context_window` is not
    // bounded.
    let cap = Some(256_000);
    let got =
        run_window_capped(tools.clone(), Some(Windows::new(Ok(Some(1_050_000)))), Some(128_000), cap)
            .await;
    assert_eq!(got, (Some(256_000), ratio.clone()));
    let got =
        run_window_capped(tools.clone(), Some(Windows::new(Ok(Some(200_000)))), Some(128_000), cap)
            .await;
    assert_eq!(got.0, Some(200_000));
    assert_eq!(run_window_capped(tools.clone(), None, Some(400_000), cap).await.0, Some(256_000));
    let windows = Windows::new(Ok(Some(1_050_000)));
    let got = run_window_capped(
        json!({ "context_window": 500_000, "tools": { "enabled": true } }),
        Some(windows.clone()),
        Some(128_000),
        cap,
    )
    .await;
    assert_eq!(got.0, Some(500_000));
    assert!(windows.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn looked_up_windows_are_cached_per_alias() {
    let windows = Windows::new(Ok(Some(200_000)));
    let cached = CachedContextWindows::new(windows.clone());
    for _ in 0..3 {
        assert_eq!(cached.context_window("llm.chat").await, Ok(Some(200_000)));
    }
    assert_eq!(cached.context_window("llm.plan").await, Ok(Some(200_000)));
    assert_eq!(
        *windows.asked.lock().unwrap(),
        vec!["llm.chat".to_string(), "llm.plan".to_string()]
    );
    // Expired: asked again.
    let windows = Windows::new(Ok(Some(200_000)));
    let cached = CachedContextWindows::with_ttl(windows.clone(), std::time::Duration::ZERO);
    cached.context_window("llm.chat").await.unwrap();
    cached.context_window("llm.chat").await.unwrap();
    assert_eq!(windows.asked.lock().unwrap().len(), 2);
    // A failure is not cached.
    let failing = Windows::new(Err("down".into()));
    let cached = CachedContextWindows::new(failing.clone());
    assert!(cached.context_window("llm.chat").await.is_err());
    assert!(cached.context_window("llm.chat").await.is_err());
    assert_eq!(failing.asked.lock().unwrap().len(), 2);
}

/// A multi-Turn Work (the built-in `work` class: Turns until
/// `report(is_end)`) with a llm_context of this test.
async fn multi_turn_work(env: &Env, llm_context: Value) -> libopendan::SessionDir {
    let mut spec = libopendan::SessionTemplate::load("work", None)
        .unwrap()
        .spec("research it");
    spec.prompt.llm_context = llm_context;
    spec.prompt.history_budget_tokens = Some(100);
    env.create_work(spec).await
}

fn multi_turn_context(window: Option<u64>) -> Value {
    let mut v = json!({
        "tools": { "enabled": true },
        "runtime": { "task_wait_ms": 300 }
    });
    if let Some(w) = window {
        v["context_window"] = json!(w);
    }
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_turn_work_compacts_at_the_window_line_and_goes_on() {
    let _serial = init_context_logger();
    std::env::set_var("LIBOPENDAN_WHO", APP);
    // The size of the run's first request (system, input, tool schemas).
    let probe_env = Env::new();
    let probe = multi_turn_work(&probe_env, multi_turn_context(None)).await;
    let first = ScriptedLlm::new(|_, _| {
        tool_call("rep", "report", json!({ "report": "probe", "is_end": true }))
    });
    let r = drive(&probe, &probe_env.deps(first.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let base = estimate_request(&ByteHeuristicTokenizer, &first.requests.lock().unwrap()[0]);

    // Window = 4 × the first request: the line is at 3.2×, the target 1.2×.
    let window = base * 4;
    let env = Env::new();
    let sd = multi_turn_work(&env, multi_turn_context(Some(window))).await;
    // About 3 × the first request of output: the next request crosses 80%.
    let bytes = base * 3 * 4;
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => tool_call(
            "c1",
            "shell",
            json!({ "command": format!("head -c {bytes} /dev/zero | tr '\\0' x") }),
        ),
        1 => {
            // Rewritten: system + the compacted session history (the huge
            // first result is summarized, at most the opening input stays).
            let all = render(&req.messages);
            assert!(req.messages.len() <= 3, "{all}");
            assert!(all.contains("SUMMARY-X"), "{all}");
            assert!(has_tool_result(req, "c1").is_none(), "{all}");
            tool_call("c2", "shell", json!({ "command": "sleep 1; echo later" }))
        }
        2 => text("The later step is still running."),
        3 => {
            assert!(last_user_text(req).contains("later"), "{}", last_user_text(req));
            tool_call("rep", "report", json!({ "report": "researched", "is_end": true }))
        }
        _ => panic!("unexpected inference {n}"),
    });
    let deps = env
        .deps(llm.clone())
        .with_summarizer(StaticSummarizer::new("SUMMARY-X"));
    let mut deps = deps;
    deps.options.max_wait = std::time::Duration::from_secs(20);
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4);

    // The rewritten request is within 30% of the window.
    let reqs = llm.requests.lock().unwrap().clone();
    let after = estimate_request(&ByteHeuristicTokenizer, &reqs[1]);
    let target = (window as f64 * 0.3).ceil() as u64;
    assert!(after <= target, "{after} > {target}");

    // One rewrite, one compaction; the same run went on into Turn 2.
    assert_eq!(rewrites(&sd), 1);
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(|e| e.body.kind() == "compaction"));
    assert_eq!(
        results_of(&sd),
        vec![
            ("c1".to_string(), 1),
            ("c2".to_string(), 1),
            ("rep".to_string(), 2)
        ]
    );
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!((st.turn_seq, st.turns_completed), (2, 2));
    assert!(!st.one_line_status.is_empty());
    let run_id = st.last_run.clone().unwrap();
    assert_eq!(sd.runs().list().unwrap(), vec![run_id.clone()]);
    let (record, snap) = sd.runs().load_checked(&run_id).unwrap();
    let snap = snap.unwrap();
    assert_eq!(record.compactions, 1);
    assert_eq!(snap.request.budget.context_window_tokens, Some(window as u32));
    // Turn 1 used two iterations; Turn 2 had the whole budget again (its
    // final report ended the run before the batch counted).
    assert_eq!(
        snap.state.tool_iterations_left,
        snap.request.tool_policy.max_tool_iterations
    );

    let events = context_log_events(sd.sid());
    let limits = events_named(&events, "llm_context.context_limit");
    assert_eq!(limits.len(), 1, "{events:?}");
    assert_ne!(limits[0].body["reason"], "provider_refused");
    let triggered: Vec<_> = events_named(&events, "opendan.context.compaction.triggered")
        .into_iter()
        .filter(|e| e.body["phase"] == "context_limit")
        .collect();
    assert_eq!(triggered.len(), 1, "{events:?}");
    assert_eq!(triggered[0].body["window_tokens"], window);
    assert_eq!(triggered[0].body["target_tokens"], target);
    assert!(triggered[0].body["prefix_tokens"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------------
// Compression by llm_compress (compaction TODO U2 / U4 / C3)
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_rewrite_fills_the_history_budget_with_head_summary_and_raw_tail() {
    let _serial = init_context_logger();
    let env = Env::new();
    let window = 60_000u64;
    let mut spec = work_spec("many large tool results");
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true },
        "context_window": window,
        "max_tool_iterations": 200
    });
    spec.prompt.history_budget_tokens = Some(1_000);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        if render(&req.messages).contains("<history_summary>") {
            return text("done after the rewrite");
        }
        let id = format!("c{n}");
        tool_call(
            &id,
            "shell",
            json!({ "command": format!("echo {id}; head -c 4000 /dev/zero | tr '\\0' x") }),
        )
    });
    // About the size of a real summary (≈ 5k tokens).
    let summarizer = StaticSummarizer::new("S".repeat(20_000));
    let deps = env.deps(llm.clone()).with_summarizer(summarizer.clone());
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");

    let reqs = llm.requests.lock().unwrap().clone();
    let rewritten = reqs
        .iter()
        .find(|r| render(&r.messages).contains("<history_summary>"))
        .expect("a rewritten request");
    // system + head (input + first round) + summary block + raw tail, every
    // tool call with its result.
    let m = &rewritten.messages;
    assert_eq!(m[0].role, AiRole::System);
    assert_eq!(m[1].role, AiRole::User);
    assert_eq!(m[2].tool_calls()[0].call_id, "c0");
    assert!(has_tool_result(rewritten, "c0").is_some());
    let summary_at = m
        .iter()
        .position(|m| m.text_content().starts_with("<history_summary>"))
        .unwrap();
    assert_eq!(summary_at, 4);
    assert!(m[4].text_content().ends_with(libopendan::runner::history::COMPRESSED_NOTE));
    assert_eq!(m[5].role, AiRole::Assistant);
    let calls: Vec<String> = m.iter().flat_map(|m| m.tool_calls()).map(|c| c.call_id).collect();
    let results: Vec<String> = m
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            AiContent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(calls, results);
    assert!(calls.len() > 2, "raw rounds stay after the summary: {calls:?}");
    // The rounds between head and tail reached the summarizer in full.
    let prompt = summarizer.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("result #c1 [ok]"), "{prompt}");

    // The history fills the budget the 30% line leaves after the prefix.
    let mut prefix = rewritten.clone();
    prefix.messages.retain(|m| m.role == AiRole::System);
    let prefix = estimate_request(&ByteHeuristicTokenizer, &prefix);
    let budget = (window as f64 * 0.3).ceil() as u64 - prefix;
    let history = estimate_request(&ByteHeuristicTokenizer, rewritten) - prefix;
    assert!(
        history as f64 >= 0.6 * budget as f64 && history <= budget,
        "history {history} of budget {budget}"
    );

    // summary.json of the rewrite: the head ends before the raw tail.
    let events = context_log_events(sd.sid());
    let compacted = events_named(&events, "opendan.context.compaction.completed");
    assert_eq!(compacted[0].body["reason"], "context_limit");
    let head_end = compacted[0].body["head_end_seq_after"].as_u64().unwrap();
    let start = compacted[0].body["start_seq_after"].as_u64().unwrap();
    assert!(head_end > 0 && head_end < start, "{:?}", compacted[0]);
    assert!(sd.summary_opt().unwrap().is_some());
    let completed = events_named(&events, "opendan.context.rewrite.completed");
    assert_eq!(completed[0].body["summarized"], true);
    assert_eq!(completed[0].body["summary_calls"], 1);
    assert!(completed[0].body["summary_tokens"].as_u64().unwrap() >= 5_000);
}

#[tokio::test]
async fn a_successful_inference_starts_the_rewrite_count_anew() {
    let _serial = init_context_logger();
    let env = Env::new();
    let sd = env.create_work(work_spec("two limits")).await;
    let llm = ScriptedLlm::fallible(|_, n| match n {
        0 => Ok(tool_call("c1", "shell", json!({ "command": "echo one" }))),
        1 | 3 => Err(refusal()),
        2 => Ok(tool_call("c2", "shell", json!({ "command": "echo two" }))),
        _ => Ok(text("done")),
    });
    let deps = env
        .deps(llm.clone())
        .with_summarizer(StaticSummarizer::new("SUMMARY-X"));
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let events = context_log_events(sd.sid());
    let attempts: Vec<Value> = events_named(&events, "opendan.context.rewrite.started")
        .iter()
        .map(|e| e.body["attempt"].clone())
        .collect();
    assert_eq!(attempts, vec![json!(1), json!(1)]);
    assert_eq!(rewrites(&sd), 2);
}
