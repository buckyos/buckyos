//! Hosting sessions in one process (xAgent §4.12, §5.2.1, §9.6): the
//! per-session runner dependencies, the driver of sub sessions and the
//! resident loop. A host never writes another session's state: everything
//! goes through that session's own `drive`, under its own lease.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_tool::xllm::XllmDeps;

use crate::bridge::EventBridge;
use crate::channel::{InputChannelFactory, Waker};
use crate::error::Result;
use crate::protocol::*;
use crate::runner::{
    drive, DriveResult, RunnerDeps, RunnerOptions, SessionAssembler, StopSignal, StopWhen,
};
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// What the sessions driven by one process share. The runtime is not among
/// them: every session gets its own, built from its configuration
/// ([`crate::runtime::runtime_for_session`]).
#[derive(Clone)]
pub struct HostDeps {
    pub who: String,
    pub agent: Arc<dyn AgentStateClient>,
    pub inputs: Arc<dyn InputChannelFactory>,
    pub waker: Arc<dyn Waker>,
    pub xllm: XllmDeps,
    pub assembler: Arc<dyn SessionAssembler>,
    /// Executable wrapped as `agent-session` in each session's bin.
    pub session_cli: Option<PathBuf>,
    pub app_tools: Vec<String>,
    pub options: RunnerOptions,
    /// Sub sessions driven at the same time.
    pub max_child_concurrency: usize,
    /// Producers started for the sessions being served.
    pub bridges: Vec<Arc<dyn EventBridge>>,
    /// Binding identity required of the sessions driven directly (not of
    /// their sub sessions).
    pub runtime_id: Option<String>,
}

impl HostDeps {
    /// Runner dependencies of one session (its own runtime, its own stop
    /// signal).
    pub fn runner_deps(&self, sd: &SessionDir, stop: StopSignal) -> Result<RunnerDeps> {
        let cfg = sd.config()?;
        let runtime = crate::runtime::runtime_for_session(
            sd,
            &cfg,
            self.agent.agent_root(),
            self.runtime_id.as_deref(),
        )?;
        let mut deps = RunnerDeps::new(
            &self.who,
            self.agent.clone(),
            self.inputs.clone(),
            runtime,
            self.xllm.clone(),
        )
        .with_waker(self.waker.clone())
        .with_assembler(self.assembler.clone())
        .with_options(self.options.clone());
        deps.session_cli = self.session_cli.clone();
        deps.app_tools = self.app_tools.clone();
        deps.stop = stop;
        Ok(deps)
    }

    async fn drive(&self, sd: &SessionDir, until: StopWhen, stop: StopSignal) -> DriveResult {
        match self.runner_deps(sd, stop) {
            Ok(deps) => drive(sd, &deps, until).await,
            Err(e) => DriveResult::BindFailed { error: e.to_json() },
        }
    }
}

struct Child {
    task: tokio::task::JoinHandle<DriveResult>,
    stop: StopSignal,
}

/// Advances the sub sessions of the sessions this process drives (and their
/// sub sessions). Each one runs under its own lease, in parallel with its
/// parent; stopping the host stops these tasks, never the committed state.
#[derive(Clone)]
pub struct ChildDriver {
    host: HostDeps,
    running: Arc<Mutex<HashMap<String, Child>>>,
    /// Last result per sub session that is not being driven right now.
    results: Arc<Mutex<HashMap<String, DriveResult>>>,
}

impl ChildDriver {
    pub fn new(mut host: HostDeps) -> Self {
        host.runtime_id = None;
        Self {
            host,
            running: Arc::new(Mutex::new(HashMap::new())),
            results: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn reap(&self) {
        let mut running = self.running.lock().expect("children");
        let done: Vec<String> = running
            .iter()
            .filter(|(_, c)| c.task.is_finished())
            .map(|(k, _)| k.clone())
            .collect();
        for sid in done {
            running.remove(&sid);
        }
    }

    /// Sub sessions being driven right now.
    pub fn busy(&self) -> usize {
        self.reap();
        self.running.lock().expect("children").len()
    }

    /// Whether the drive of `e` could advance it now.
    fn advanceable(&self, e: &RegistryEntry, parent_stopped: bool) -> bool {
        if e.status.run_state == RunState::Finished {
            // A queued `decide` is applied by the driver after the finish.
            return e.status.pending_decision.is_some()
                && !matches!(
                    self.results.lock().expect("results").get(&e.session_id),
                    Some(DriveResult::Finished { .. })
                );
        }
        if parent_stopped {
            return true;
        }
        match self.results.lock().expect("results").get(&e.session_id) {
            // Blocked for a reason another drive cannot remove.
            Some(
                DriveResult::NotDriver { .. }
                | DriveResult::Unregistered
                | DriveResult::BindFailed { .. }
                | DriveResult::RecoveryBlocked(_),
            ) => false,
            // Waiting for input it has no channel for: only its parent's
            // answer (a new session input) can change that.
            _ => !(e.status.run_state == RunState::Waiting
                && e.status.waiting_for == Some(WaitingKind::Input)
                && e.input_queue.is_none()),
        }
    }

    /// One look at the registry: every sub session of `roots` (transitively)
    /// that this identity drives, is not finished and is not being driven
    /// gets a drive. `true`: something is running or was started.
    pub async fn tick(&self, roots: &[String]) -> Result<bool> {
        self.reap();
        let mut parents: Vec<String> = roots.to_vec();
        let mut seen = 0;
        while seen < parents.len() {
            let level: Vec<String> = parents[seen..].to_vec();
            seen = parents.len();
            for e in self.host.agent.sessions().children_of(&level).await? {
                if !parents.contains(&e.session_id) {
                    parents.push(e.session_id.clone());
                }
                if e.driver.principal != self.host.who {
                    continue;
                }
                let parent_stopped = match e.origin.as_ref().and_then(|o| o.parent_session.clone())
                {
                    Some(p) => self
                        .host
                        .agent
                        .sessions()
                        .lookup(&p)
                        .await?
                        .is_some_and(|p| p.status.outcome == Some(Outcome::Stopped)),
                    None => false,
                };
                {
                    let running = self.running.lock().expect("children");
                    if let Some(c) = running.get(&e.session_id) {
                        // A stopped parent stops the sub sessions this host
                        // drives (a sub session without a queue has no other
                        // way to hear it).
                        if parent_stopped {
                            c.stop.request();
                        }
                        continue;
                    }
                    if running.len() >= self.host.max_child_concurrency.max(1) {
                        continue;
                    }
                }
                if !self.advanceable(&e, parent_stopped) {
                    continue;
                }
                let Ok(sd) = SessionDir::open(&e.location) else {
                    continue;
                };
                let stop = StopSignal::default();
                if parent_stopped && e.status.run_state != RunState::Finished {
                    stop.request();
                }
                let host = self.host.clone();
                let results = self.results.clone();
                let child_stop = stop.clone();
                let sid = e.session_id.clone();
                let task = tokio::spawn(async move {
                    let r = host.drive(&sd, StopWhen::Idle, child_stop).await;
                    results.lock().expect("results").insert(sid, r.clone());
                    r
                });
                self.running
                    .lock()
                    .expect("children")
                    .insert(e.session_id.clone(), Child { task, stop });
            }
        }
        Ok(self.busy() > 0)
    }

    /// Sub sessions of `roots` (transitively) that are not finished.
    pub async fn unfinished(&self, roots: &[String]) -> Result<Vec<String>> {
        let mut parents: Vec<String> = roots.to_vec();
        let mut out = Vec::new();
        let mut seen = 0;
        while seen < parents.len() {
            let level: Vec<String> = parents[seen..].to_vec();
            seen = parents.len();
            for e in self.host.agent.sessions().children_of(&level).await? {
                if !parents.contains(&e.session_id) {
                    parents.push(e.session_id.clone());
                    if e.status.run_state != RunState::Finished {
                        out.push(e.session_id.clone());
                    }
                }
            }
        }
        Ok(out)
    }

    /// Keep driving until no sub session of `roots` is running or can be
    /// advanced, or `max` passed.
    pub async fn settle(&self, roots: &[String], max: Duration) -> Result<()> {
        let started = Instant::now();
        loop {
            let active = self.tick(roots).await?;
            if !active || started.elapsed() >= max {
                return Ok(());
            }
            tokio::time::sleep(self.host.options.poll_interval.min(Duration::from_millis(200)))
                .await;
        }
    }

    /// Stop the driving tasks of this process (committed state stays).
    pub async fn shutdown(&self) {
        let children: Vec<Child> = self
            .running
            .lock()
            .expect("children")
            .drain()
            .map(|(_, c)| c)
            .collect();
        for c in children {
            c.task.abort();
            let _ = c.task.await;
        }
    }
}

/// Result of [`run_session`].
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub result: DriveResult,
    /// Sub sessions that are not finished when the call returns: registered
    /// and committed, to be taken over by a resident host.
    pub children: Vec<String>,
}

/// Drive one session until `until`, advancing the sub sessions it creates
/// in the same process. Afterwards the sub sessions are driven on until
/// they are idle or finished, unless `detach_children`.
pub async fn run_session(
    host: &HostDeps,
    sd: &SessionDir,
    until: StopWhen,
    stop: StopSignal,
    detach_children: bool,
) -> RunOutcome {
    let children = ChildDriver::new(host.clone());
    let roots = vec![sd.sid().to_string()];
    // Sub sessions are registered by tool calls while the parent runs: the
    // registry is polled independently of the parent's commits.
    let ticker = {
        let children = children.clone();
        let roots = roots.clone();
        let pause = host.options.poll_interval.min(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                if let Err(e) = children.tick(&roots).await {
                    log::warn!("sub session driver: {e}");
                }
                tokio::time::sleep(pause).await;
            }
        })
    };
    let result = host.drive(sd, until, stop).await;
    ticker.abort();
    let _ = ticker.await;
    if !detach_children {
        if let Err(e) = children.settle(&roots, host.options.max_wait).await {
            log::warn!("sub session driver: {e}");
        }
    }
    children.shutdown().await;
    let unfinished = children.unfinished(&roots).await.unwrap_or_default();
    RunOutcome {
        result,
        children: unfinished,
    }
}

/// Whether an idle session still has work only a living host advances
/// (waiting on tools / sub sessions, followed tasks, pulled subscriptions).
fn has_pending_work(sd: &SessionDir) -> bool {
    let (Ok(state), Ok(cfg)) = (sd.state(), sd.config()) else {
        return false;
    };
    !state.watched_tasks.is_empty()
        || state
            .waiting_for
            .as_ref()
            .is_some_and(|w| matches!(w.kind, WaitingKind::Tool | WaitingKind::Children))
        || cfg
            .subscriptions
            .iter()
            .any(|s| matches!(s.source, SubscriptionSource::Session { .. }))
}

struct BridgeSet {
    rev: u64,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl BridgeSet {
    fn stop(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

fn start_bridges(host: &HostDeps, cfg: &SessionConfig) -> BridgeSet {
    let mut tasks = Vec::new();
    if cfg.channels.kmsg().is_some() {
        for b in host.bridges.iter().filter(|b| b.wants(cfg)) {
            let b = b.clone();
            let cfg = cfg.clone();
            tasks.push(tokio::spawn(async move {
                let sid = cfg.session.session_id.clone();
                if let Err(e) = b.run(cfg).await {
                    log::warn!("event bridge of {sid}: {e}");
                }
            }));
        }
    }
    BridgeSet {
        rev: cfg.config_rev,
        tasks,
    }
}

/// Serve one session until it finishes, is stopped, cannot be driven by
/// this host, or was idle without pending work for `idle_unload`.
async fn serve_one(
    host: HostDeps,
    sd: SessionDir,
    stop: StopSignal,
    idle_unload: Option<Duration>,
) -> DriveResult {
    let mut bridges: Option<BridgeSet> = None;
    let mut idle_since: Option<Instant> = None;
    let mut backoff = host.options.poll_interval;
    let result = loop {
        if let Ok(cfg) = sd.config() {
            // Bridges live across drives; a changed configuration
            // (subscriptions) restarts them.
            if bridges.as_ref().map(|b| b.rev) != Some(cfg.config_rev) {
                if let Some(mut b) = bridges.take() {
                    b.stop();
                }
                bridges = Some(start_bridges(&host, &cfg));
            }
        }
        let r = host.drive(&sd, StopWhen::Idle, stop.clone()).await;
        let pause = match &r {
            DriveResult::Finished { .. } => {
                // Controls queued for a finished session (decide) were
                // applied by this drive.
                break r;
            }
            DriveResult::NotDriver { .. }
            | DriveResult::Unregistered
            | DriveResult::BindFailed { .. }
            | DriveResult::RecoveryBlocked(_) => break r,
            DriveResult::Busy { .. } | DriveResult::RunBusy { .. } | DriveResult::LeaseLost => {
                backoff = (backoff * 2).min(Duration::from_secs(30));
                backoff
            }
            DriveResult::Error { error, .. } => {
                let retry = error.get("recoverable").and_then(|v| v.as_bool()).unwrap_or(false);
                log::warn!("session {}: {error}", sd.sid());
                if !retry {
                    break r;
                }
                backoff = (backoff * 2).min(Duration::from_secs(60));
                backoff
            }
            _ => {
                backoff = host.options.poll_interval;
                if has_pending_work(&sd) {
                    idle_since = None;
                } else {
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    if idle_unload.is_some_and(|d| since.elapsed() >= d) {
                        break r;
                    }
                }
                host.options.poll_interval
            }
        };
        if stop.requested() {
            break r;
        }
        let wake = sd.config().ok().and_then(|c| c.channels.wake_event);
        tokio::select! {
            _ = host.waker.wait(wake.as_deref(), pause) => {}
            _ = stop.wait() => {}
        }
    };
    if let Some(mut b) = bridges {
        b.stop();
    }
    result
}

/// Resident hosting (§9.6): every target is served by its own loop
/// (`drive(Idle)` → notification or bounded poll → `drive(Idle)`), their sub
/// sessions by one [`ChildDriver`]. Returns when every target ended its
/// loop; `stop` ends them (each session is stopped through its own drive).
pub async fn serve(
    host: &HostDeps,
    targets: Vec<SessionDir>,
    stop: StopSignal,
    idle_unload: Option<Duration>,
) -> Vec<(String, DriveResult)> {
    let children = ChildDriver::new(host.clone());
    let roots: Vec<String> = targets.iter().map(|sd| sd.sid().to_string()).collect();
    let ticker = {
        let children = children.clone();
        let roots = roots.clone();
        let pause = host.options.poll_interval.min(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                if let Err(e) = children.tick(&roots).await {
                    log::warn!("sub session driver: {e}");
                }
                tokio::time::sleep(pause).await;
            }
        })
    };
    let mut loops = Vec::new();
    for sd in targets {
        let sid = sd.sid().to_string();
        let task = tokio::spawn(serve_one(host.clone(), sd, stop.clone(), idle_unload));
        loops.push((sid, task));
    }
    let mut out = Vec::new();
    for (sid, task) in loops {
        let r = task.await.unwrap_or(DriveResult::Error {
            rev: 0,
            error: serde_json::json!({ "kind": "host", "message": "the serving task ended abnormally" }),
        });
        out.push((sid, r));
    }
    // Sub sessions of a stopped parent are stopped by their own drives.
    if let Err(e) = children.settle(&roots, host.options.max_wait.min(Duration::from_secs(30))).await {
        log::warn!("sub session driver: {e}");
    }
    ticker.abort();
    let _ = ticker.await;
    children.shutdown().await;
    out
}
