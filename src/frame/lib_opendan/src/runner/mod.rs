//! Session Runner (§8) — the Rust reference implementation. **Not a
//! protocol**: other languages (buckyos-websdk ts-runner) implement the same
//! directory, commit and lock semantics with their own structure. The one
//! protocol piece used here is xllm's run directory (§8.7).

pub mod assembler;
mod children;
mod drive;
pub mod input_view;
mod flush;
pub mod history;
mod hook;
mod inputs;
mod live;
mod outbound;
mod outcome;
mod receipts;
mod reports;
mod reconcile;
mod rounds;
mod shared;
mod tools;
mod turn_task;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{InputChannelFactory, NoopNotifier, Notifier, PollWaker, Waker};
use crate::error::RecoveryBlocked;
use crate::protocol::*;
use crate::runtime::AgentRuntime;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

pub use assembler::{
    render_template, BehaviorAssembler, DefaultAssembler, InputMaterial, SessionAssembler,
};
pub use children::{SessionTaskResolver, SESSION_TASK_PREFIX};
pub use drive::drive;
pub use outbound::{
    compose_text, has_pending_outbound, OutboundSink, SendResult, TurnReply, AGENT_TASK_META,
};
pub use turn_task::{
    has_pending_turn_tasks, TurnTaskEnd, TurnTaskOpen, TurnTaskSink, TurnTaskStatus,
};
pub use history::{LlmSummarizer, Summarizer};
pub use tools::classify_effect;

/// When `drive` returns (a property of this call, not a session end
/// condition).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopWhen {
    /// Return when there is nothing to do (resident hosting / UI).
    Idle,
    /// Advance a work session until it finishes.
    Finished,
    /// Return after the drive loop handled `n` `LLMContext` outcomes (one
    /// per run segment started or resumed by this drive, whatever its kind:
    /// done, behavior switch, suspension, error). Not a count of Rounds
    /// (inferences), `run()` calls (a context-limit rewrite runs the context
    /// again inside one segment) or Turns. Checked before each segment:
    /// `n = 0` only recovers and applies control inputs. The drive still
    /// returns earlier when the session finishes (`Finished`), stops on an
    /// error (`Error`), or has nothing to run (`OutcomesHandled`, which then
    /// reports the waiting state).
    MaxOutcomes { n: u64 },
    /// Return when a Turn was closed by this drive (its recovery included),
    /// with that Turn's result. A return condition only: what closes a Turn
    /// is unchanged.
    TurnClosed,
}

/// Why `drive` returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriveResult {
    /// Nothing more to do right now (idle / waiting for input).
    Idle {
        rev: u64,
        run_state: RunState,
    },
    /// The session is finished (`finished` state).
    Finished {
        rev: u64,
        outcome: Option<Outcome>,
        acceptance: Acceptance,
    },
    /// `StopWhen::MaxOutcomes` reached, or nothing left to run before it.
    OutcomesHandled {
        rev: u64,
        run_state: RunState,
    },
    /// `StopWhen::TurnClosed`: a Turn was closed by this drive (the session
    /// may have finished with it).
    TurnClosed {
        rev: u64,
        turn: u64,
        status: TurnStatus,
        answer: Option<String>,
    },
    /// `StopWhen::TurnClosed`: the open Turn could not be closed by this
    /// drive (it waits for input, a tool or sub sessions).
    TurnOpen {
        rev: u64,
        turn: u64,
        waiting_for: Option<WaitingFor>,
    },
    /// Another holder advances the session (display info).
    Busy {
        holder: Option<Value>,
    },
    /// The run is executed by someone else (e.g. xllm took it over).
    RunBusy {
        run_id: String,
    },
    NotDriver {
        driver: String,
    },
    Unregistered,
    BindFailed {
        error: Value,
    },
    RecoveryBlocked(RecoveryBlocked),
    LeaseLost,
    /// The run stopped on an error; state carries `last_error`.
    Error {
        rev: u64,
        error: Value,
    },
}

impl DriveResult {
    pub fn is_finished(&self) -> bool {
        matches!(self, DriveResult::Finished { .. })
    }
}

/// Tunables of the reference runner.
#[derive(Debug, Clone)]
pub struct RunnerOptions {
    /// Polling fallback when kevent wake-ups are lost.
    pub poll_interval: Duration,
    /// `until = finished`: give up waiting for input after this long.
    pub max_wait: Duration,
    /// History budget (tokens) when `prompt.history_budget_tokens` is unset.
    pub history_budget_tokens: u32,
    pub compact_ratio: f32,
    /// Throttle of activity heartbeat commits during a run.
    pub heartbeat_interval: Duration,
    /// Active sessions rendered into a turn.
    pub active_sessions_limit: usize,
    /// Semi-subscription state versions shown per snapshot message; the
    /// rest stays pending.
    pub change_budget: usize,
    /// msg / Input events one `on_input` batch takes (`input.mode = batch`).
    pub input_batch_max: usize,
    pub load_hints: bool,
    /// Snapshots kept per finished run.
    pub keep_snapshots: usize,
    /// How long a Turn answering a message stays open before its
    /// placeholder is sent (a first tool call sends it earlier).
    pub placeholder_delay: Duration,
}

/// A stop requested by the driving process itself (SIGINT, a host stopping
/// a sub session without a queue): handled like a queued `stop` — the run is
/// interrupted, the Turn closes as stopped and the session finishes.
#[derive(Clone, Default)]
pub struct StopSignal {
    flag: Arc<std::sync::atomic::AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl StopSignal {
    pub fn request(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn requested(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns once a stop is requested.
    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.requested() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for RunnerOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            max_wait: Duration::from_secs(600),
            history_budget_tokens: 24_000,
            compact_ratio: 0.75,
            heartbeat_interval: Duration::from_secs(60),
            active_sessions_limit: 8,
            change_budget: 16,
            input_batch_max: 16,
            load_hints: true,
            keep_snapshots: 2,
            placeholder_delay: Duration::from_secs(3),
        }
    }
}

/// Everything a runner needs (§8.1).
#[derive(Clone)]
pub struct RunnerDeps {
    /// Driving identity = the driving process's app principal (Q7 / Q16).
    pub who: String,
    pub runner_id: String,
    pub agent: Arc<dyn AgentStateClient>,
    pub inputs: Arc<dyn InputChannelFactory>,
    pub waker: Arc<dyn Waker>,
    pub runtime: Arc<dyn AgentRuntime>,
    /// xllm deps: LLM client factory (called with the session's provider
    /// config, as `who`), host tools. The bash runner is set per run.
    pub xllm: XllmDeps,
    pub assembler: Arc<dyn SessionAssembler>,
    pub summarizer: Option<Arc<dyn Summarizer>>,
    pub notifier: Arc<dyn Notifier>,
    /// Where replies leave the process (`None`: no outbox is kept).
    pub outbound: Option<Arc<dyn OutboundSink>>,
    /// The host's task service (`None`: Turns get no task).
    pub turn_tasks: Option<Arc<dyn TurnTaskSink>>,
    /// Executable wrapped as `agent-session` in `.runtime/bin`.
    pub session_cli: Option<PathBuf>,
    /// Tools that only exist in this runner process (`requirement.app_tools`).
    pub app_tools: Vec<String>,
    pub options: RunnerOptions,
    pub stop: StopSignal,
}

impl RunnerDeps {
    pub fn new(
        who: &str,
        agent: Arc<dyn AgentStateClient>,
        inputs: Arc<dyn InputChannelFactory>,
        runtime: Arc<dyn AgentRuntime>,
        xllm: XllmDeps,
    ) -> Self {
        Self {
            who: who.to_string(),
            runner_id: crate::ids::new_runner_id(),
            agent,
            inputs,
            waker: Arc::new(PollWaker),
            runtime,
            xllm,
            assembler: Arc::new(DefaultAssembler::default()),
            summarizer: None,
            notifier: Arc::new(NoopNotifier),
            outbound: None,
            turn_tasks: None,
            session_cli: None,
            app_tools: Vec::new(),
            options: RunnerOptions::default(),
            stop: StopSignal::default(),
        }
    }

    pub fn with_waker(mut self, w: Arc<dyn Waker>) -> Self {
        self.waker = w;
        self
    }

    pub fn with_assembler(mut self, a: Arc<dyn SessionAssembler>) -> Self {
        self.assembler = a;
        self
    }

    pub fn with_summarizer(mut self, s: Arc<dyn Summarizer>) -> Self {
        self.summarizer = Some(s);
        self
    }

    pub fn with_notifier(mut self, n: Arc<dyn Notifier>) -> Self {
        self.notifier = n;
        self
    }

    pub fn with_session_cli(mut self, p: PathBuf) -> Self {
        self.session_cli = Some(p);
        self
    }

    pub fn with_options(mut self, o: RunnerOptions) -> Self {
        self.options = o;
        self
    }

    pub fn holder(&self) -> HolderInfo {
        HolderInfo {
            runner_id: self.runner_id.clone(),
            principal: self.who.clone(),
            host: Some(crate::runtime::native_host_id()),
            pid: std::process::id(),
            runtime_id: Some(self.runtime.descriptor().runtime_id.clone()),
        }
    }
}

/// Convenience wrapper.
pub struct SessionRunner {
    pub deps: RunnerDeps,
}

impl SessionRunner {
    pub fn new(deps: RunnerDeps) -> Self {
        Self { deps }
    }

    pub async fn drive(&self, sd: &SessionDir, until: StopWhen) -> DriveResult {
        drive(sd, &self.deps, until).await
    }
}
