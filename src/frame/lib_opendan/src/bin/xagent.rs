//! xagent — drive an Agent Session for one Turn (or keep driving it).
//!
//! The layer above xllm: xllm advances one LLMContext run to an outcome,
//! xagent loads (or creates) an Agent Session and advances it until a Turn
//! closes, or serves it. Design: `doc/opendan/xAgent.md`.
//!
//! The same executable is wrapped as `agent-session` in every session's
//! `.runtime/bin`: commands run by `shell` use it to reach their session
//! and the Agent State.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use agent_tool::{AgentToolResult, AgentToolStatus};
use buckyos_api::msg_queue::MsgQueueClient;
use libopendan::api::{create_session, create_sub_session, SubSessionSpec, SubWorkspace};
use libopendan::bridge::{EventBridge, KEventBridge, TimerBridge};
use libopendan::channel::{InputChannelFactory, InputSource, KEventWaker, KmsgChannels, PollWaker, Waker};
use libopendan::host::{run_session, serve, HostDeps};
use libopendan::protocol::*;
use libopendan::runner::{
    BehaviorAssembler, DriveResult, RunnerOptions, StopSignal, StopWhen, SESSION_TASK_PREFIX,
};
use libopendan::state::{connect, AgentStateClient, ConnectOptions, StateLocator};
use libopendan::{InputChannel, OpenDanError, SessionDir, SessionTemplate};
use serde_json::{json, Value};

const USAGE: &str = "\
xagent — drive an Agent Session for one Turn (or keep driving it)

  xagent new    --agent <did> --objective <text> [--class work|ui|self_improve|self_check] [--parent <dir>]
                [--behavior <name>] [--llm-context <json|@file>] [--runtime <id>] [--workspace <path>]
                [--subscribe <spec>]... [--msg <text>] [--key <idem>]
                [--until turn|finished|idle|outcomes:<n>] [--no-run]
  xagent run    <session_dir|sid> [--msg <text> | --msg-file <path> | --event <json|@file>]
                [--until turn|finished|idle|outcomes:<n>] [--runtime <id>] [--max-wait <secs>]
                [--no-bridge] [--detach-children]
  xagent serve  <session_dir|sid>... [--idle-unload <secs>] [--no-bridge]
  xagent post   <sid> (--msg <text> [--from <did>] [--attach <obj_id>[=<name>]]... [--reply-to <obj_id>] | --json <file | ->)
  xagent ctl    <sid> (stop | decide accept|discard [--note <t>] | subscribe <spec> | unsubscribe <id>
                       | activity [--summary <t>] [--touch <ref>]... [--clear] | perceive <text>)
  xagent status <sid> [--worklog <n>] [--report] [--run] [--events]
  xagent list   --agent <did> [--active]
  xagent behaviors --agent <did> [--frozen <sid>]
  xagent xllm   <sid> [--run <id>]
  xagent schema <dir>

  <spec> = active|semi:object:<id>[#<event>] | semi:session:<sid>[:watch=f1,f2] | active|semi:timer:<name>

  inside a session's shell (as `agent-session`; the session comes from the environment):
  ctl activity ... | ctl perceive <text> | recall <tag>... | note <text> | sessions [--active] [--children]
  read-session <sid> [--report] [--worklog <n>] | artifact head <aid> | artifact list
  create-worksession --objective <t> [--msg <t>]... [--attach <obj_id>[=<name>]]... [--context recent:<n>|none]
        [--class <c>] [--behavior <b>] [--workspace inherit|new|<id>] [--runtime inherit|<id>]
        [--report final|progress|none] [--interactive] [--wait [--wait-ms <ms>]]
  wait <sid> [--wait-ms <ms>] | post <sid> --msg <t>

identity and location (flags or environment):
  --agent <did>        [$OPENDAN_AGENT_DID]     --who <principal>  [$LIBOPENDAN_WHO]
  --agent-root <dir>   [$OPENDAN_AGENT_ROOT]    --state-url <url>  [$OPENDAN_AGENT_STATE_URL]
  --queue-dir <dir>    [$LIBOPENDAN_QUEUE_DIR]  development file queue; without it the zone's kmsg

exit codes: 0 completed | 1 failed | 2 bad arguments / configuration | 3 not finished (idle, open Turn, input_full)
            4 stopped | 5 busy | 6 blocked (not driver, unregistered, bind, runtime mismatch, recovery, readonly)
";

/// A failure with its exit code.
struct Fail {
    code: i32,
    msg: String,
}

type R<T> = Result<T, Fail>;

fn bad(msg: impl Into<String>) -> Fail {
    Fail {
        code: 2,
        msg: msg.into(),
    }
}

impl From<OpenDanError> for Fail {
    fn from(e: OpenDanError) -> Self {
        let code = match &e {
            OpenDanError::InvalidArgument(_)
            | OpenDanError::NotFound(_)
            | OpenDanError::Json { .. }
            | OpenDanError::SessionIdConflict(_)
            | OpenDanError::Channel(_) => 2,
            OpenDanError::InputFull { .. } => 3,
            OpenDanError::Busy { .. } | OpenDanError::RunBusy { .. } | OpenDanError::LeaseLost(_) => 5,
            OpenDanError::NotDriver { .. }
            | OpenDanError::Unregistered(_)
            | OpenDanError::Bind(_)
            | OpenDanError::RuntimeMismatch { .. }
            | OpenDanError::RecoveryBlocked(_)
            | OpenDanError::SessionReadonly { .. } => 6,
            _ => 1,
        };
        Fail {
            code,
            msg: e.to_string(),
        }
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        bad(e.to_string())
    }
}

impl From<serde_json::Error> for Fail {
    fn from(e: serde_json::Error) -> Self {
        bad(e.to_string())
    }
}

/// Flags (options without a value) of every command.
const FLAGS: &[&str] = &[
    "no-run",
    "no-bridge",
    "detach-children",
    "report",
    "events",
    "active",
    "children",
    "interactive",
    "wait",
    "clear",
    "help",
];

struct Args {
    pos: VecDeque<String>,
    opts: Vec<(String, Option<String>)>,
}

impl Args {
    /// `flags`: additional value-less options of the command being parsed.
    fn parse(argv: Vec<String>, flags: &[&str]) -> Self {
        let mut pos = VecDeque::new();
        let mut opts = Vec::new();
        let mut it = argv.into_iter();
        while let Some(a) = it.next() {
            match a.strip_prefix("--") {
                Some(name) if !name.is_empty() => {
                    let flag = FLAGS.contains(&name) || flags.contains(&name);
                    let val = if flag { None } else { it.next() };
                    opts.push((name.to_string(), val));
                }
                _ => pos.push_back(a),
            }
        }
        Self { pos, opts }
    }

    fn get(&self, name: &str) -> Option<String> {
        self.opts
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.clone())
    }

    fn all(&self, name: &str) -> Vec<String> {
        self.opts
            .iter()
            .filter(|(n, _)| n == name)
            .filter_map(|(_, v)| v.clone())
            .collect()
    }

    fn has(&self, name: &str) -> bool {
        self.opts.iter().any(|(n, _)| n == name)
    }

    fn env_or(&self, name: &str, env: &str) -> Option<String> {
        self.get(name)
            .or_else(|| std::env::var(env).ok())
            .filter(|v| !v.is_empty())
    }

    fn next(&mut self, what: &str) -> R<String> {
        self.pos
            .pop_front()
            .ok_or_else(|| bad(format!("missing {what}")))
    }

    fn num<T: std::str::FromStr>(&self, name: &str) -> R<Option<T>> {
        self.get(name)
            .map(|v| v.parse().map_err(|_| bad(format!("bad --{name} {v}"))))
            .transpose()
    }
}

fn print(v: &impl serde::Serialize) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn who(a: &Args) -> String {
    a.env_or("who", "LIBOPENDAN_WHO")
        .unwrap_or_else(|| "app:xagent@local".to_string())
}

fn file_or_inline(s: &str) -> R<String> {
    Ok(match s.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)?,
        None => s.to_string(),
    })
}

/// No queue client: sessions without an input queue can still be created
/// and driven; a session that has one needs `--queue-dir` or the zone's kmsg.
struct NoQueue;

#[async_trait::async_trait]
impl InputChannelFactory for NoQueue {
    async fn open(&self, cfg: &SessionConfig) -> libopendan::Result<Vec<Arc<dyn InputSource>>> {
        if cfg.channels.inputs.is_empty() {
            return Ok(Vec::new());
        }
        Err(OpenDanError::Channel(format!(
            "session {} has an input queue but this host has no queue client (--queue-dir, or run inside a BuckyOS zone)",
            cfg.session.session_id
        )))
    }

    fn queue_client(&self) -> Option<Arc<MsgQueueClient>> {
        None
    }
}

/// The process's connection to an agent: state, channels, identity.
struct Ctx {
    who: String,
    agent: Arc<dyn AgentStateClient>,
    channels: Arc<dyn InputChannelFactory>,
    waker: Arc<dyn Waker>,
    kevent: Option<Arc<buckyos_api::KEventClient>>,
}

/// `queue`: the command may need a queue client (posting, driving).
async fn ctx(a: &Args, agent_did: Option<String>, queue: bool) -> R<Ctx> {
    let who = who(a);
    let did = agent_did
        .or_else(|| a.env_or("agent", "OPENDAN_AGENT_DID"))
        .ok_or_else(|| bad("missing --agent (or $OPENDAN_AGENT_DID)"))?;
    let mut waker: Arc<dyn Waker> = Arc::new(PollWaker);
    let mut kevent = None;
    let (channels, client): (Arc<dyn InputChannelFactory>, Option<Arc<MsgQueueClient>>) =
        match a.env_or("queue-dir", "LIBOPENDAN_QUEUE_DIR") {
            Some(dir) => {
                let ch = Arc::new(KmsgChannels::dir(&dir)?);
                let client = ch.client();
                (ch, Some(client))
            }
            None if queue => match zone_clients().await {
                Some((client, kev)) => {
                    waker = Arc::new(KEventWaker::new(kev.clone()));
                    kevent = Some(kev);
                    (Arc::new(KmsgChannels::new(client.clone())), Some(client))
                }
                None => (Arc::new(NoQueue), None),
            },
            None => (Arc::new(NoQueue), None),
        };
    let hint = match (
        a.env_or("agent-root", libopendan::state::ENV_AGENT_ROOT),
        a.env_or("state-url", libopendan::state::ENV_AGENT_STATE_URL),
    ) {
        (Some(root), _) => Some(StateLocator::AgentRoot(PathBuf::from(root))),
        (None, Some(endpoint)) => Some(StateLocator::Krpc { endpoint }),
        (None, None) => None,
    };
    let agent = connect(
        &did,
        &who,
        ConnectOptions {
            hint,
            queue: client,
            waker: Some(waker.clone()),
        },
    )
    .await?;
    Ok(Ctx {
        who,
        agent,
        channels,
        waker,
        kevent,
    })
}

/// kmsg and kevent of the zone this process runs in, when there is one.
async fn zone_clients() -> Option<(Arc<MsgQueueClient>, Arc<buckyos_api::KEventClient>)> {
    if let Err(e) = agent_tool::xllm::ensure_buckyos_runtime().await {
        log::info!("no BuckyOS runtime ({e}); sessions with an input queue need --queue-dir");
        return None;
    }
    let rt = buckyos_api::get_buckyos_api_runtime().ok()?;
    let queue = rt.get_msg_queue_client().await.ok()?;
    let kevent = rt.get_kevent_client().await.ok()?;
    Some((Arc::new(queue), Arc::new(kevent)))
}

/// A session named by its directory or its id.
async fn locate(a: &Args, target: &str, queue: bool) -> R<(Ctx, SessionDir)> {
    if Path::new(target).join(STATE_DIR).is_dir() {
        let sd = SessionDir::open(target)?;
        let did = sd.config()?.session.agent_did;
        return Ok((ctx(a, Some(did), queue).await?, sd));
    }
    let c = ctx(a, None, queue).await?;
    let entry = c
        .agent
        .sessions()
        .lookup(target)
        .await?
        .ok_or_else(|| bad(format!("session {target} is not registered (and is not a session directory)")))?;
    let sd = SessionDir::open(&entry.location)?;
    Ok((c, sd))
}

fn parse_until(a: &Args) -> R<StopWhen> {
    Ok(match a.get("until").as_deref() {
        None | Some("turn") => StopWhen::TurnClosed,
        Some("finished") => StopWhen::Finished,
        Some("idle") => StopWhen::Idle,
        Some(s) if s.starts_with("outcomes:") => StopWhen::MaxOutcomes {
            n: s[9..].parse().map_err(|_| bad(format!("bad --until {s}")))?,
        },
        Some(s) => return Err(bad(format!("bad --until {s} (turn | finished | idle | outcomes:<n>)"))),
    })
}

fn parse_subscription(spec: &str) -> R<Subscription> {
    let err = || bad(format!("bad subscription `{spec}` (active|semi:object:<id>[#<event>] | semi:session:<sid>[:watch=f1,f2] | active|semi:timer:<name>)"));
    let (mode, rest) = spec.split_once(':').ok_or_else(err)?;
    let mode = match mode {
        "active" => SubscriptionMode::Active,
        "semi" => SubscriptionMode::Semi,
        _ => return Err(err()),
    };
    let (kind, rest) = rest.split_once(':').ok_or_else(err)?;
    let mut watch = Vec::new();
    let source = match kind {
        "session" => {
            // Pulled session state is always observed; an Input needs a
            // persistent candidate, which only sub sessions have.
            if mode == SubscriptionMode::Active {
                return Err(bad(format!(
                    "`{spec}`: a session subscription is pulled from the registry and can only be semi; sub sessions report to their parent by themselves"
                )));
            }
            let (sid, w) = match rest.split_once(":watch=") {
                Some((sid, w)) => (sid, Some(w)),
                None => (rest, None),
            };
            watch = w
                .map(|w| w.split(',').map(str::to_string).collect())
                .unwrap_or_default();
            SubscriptionSource::Session {
                session_ref: sid.to_string(),
            }
        }
        "object" => {
            let (object, event) = rest.split_once('#').unwrap_or((rest, ""));
            SubscriptionSource::ObjectEvent {
                object: object.to_string(),
                event: event.to_string(),
            }
        }
        "timer" => SubscriptionSource::Timer {
            name: rest.to_string(),
        },
        _ => return Err(err()),
    };
    if rest.is_empty() {
        return Err(err());
    }
    Ok(Subscription {
        id: format!("sub-{}", libopendan::ids::h(&[spec])),
        mode,
        source,
        watch,
    })
}

/// A message built by the construction helpers (`--msg` and its options).
fn msg_input(c: &Ctx, a: &Args, text: String) -> R<PostedInput> {
    let from = match a.get("from") {
        Some(d) => parse_did(&d)?,
        None => did_of_principal(&c.who)?,
    };
    let agent = parse_did(c.agent.agent_did())?;
    let mut msg = text_msg(&from, &agent, text);
    for att in a.all("attach") {
        let (id, name) = match att.split_once('=') {
            Some((id, name)) => (id.to_string(), Some(name.to_string())),
            None => (att.clone(), None),
        };
        let obj_id = ndn_lib::ObjId::new(&id).map_err(|e| {
            bad(format!("--attach takes the ObjId of a data object (register local files in the NamedStore first): {e}"))
        })?;
        msg = attach(msg, obj_id, name);
    }
    if let Some(r) = a.get("reply-to") {
        msg = reply_to(
            msg,
            ndn_lib::ObjId::new(&r).map_err(|e| bad(format!("--reply-to: {e}")))?,
        );
    }
    Ok(PostedInput::msg(&c.who, msg, MsgDelivery::default())?)
}

/// The Agent input a command line carries (`--msg`, `--msg-file`,
/// `--event`, `--json`): msg / event only — control goes through `ctl`.
fn agent_input(c: &Ctx, a: &Args) -> R<Option<PostedInput>> {
    let input = if let Some(src) = a.get("json") {
        let text = if src == "-" {
            std::io::read_to_string(std::io::stdin())?
        } else {
            std::fs::read_to_string(&src)?
        };
        PostedInput::from_json(serde_json::from_str(&text)?, &c.who)?
    } else if let Some(src) = a.get("event") {
        let mut v: Value = serde_json::from_str(&file_or_inline(&src)?)?;
        if v.get("type").is_none() {
            v = json!({ "type": "event", "key": v.get("key").cloned().unwrap_or(Value::Null), "payload": v });
        }
        PostedInput::from_json(v, &c.who)?
    } else if let Some(path) = a.get("msg-file") {
        msg_input(c, a, std::fs::read_to_string(path)?)?
    } else if let Some(t) = a.get("msg") {
        msg_input(c, a, t)?
    } else {
        return Ok(None);
    };
    if matches!(input.input, SessionInput::Control(_)) {
        return Err(bad(
            "control records are not Agent input: use `xagent ctl <sid> ...`",
        ));
    }
    Ok(Some(input))
}

fn host(c: &Ctx, a: &Args) -> R<HostDeps> {
    let mut options = RunnerOptions::default();
    if let Some(secs) = a.num::<u64>("max-wait")? {
        options.max_wait = Duration::from_secs(secs);
    }
    if let Some(ms) = a.num::<u64>("poll-ms")? {
        options.poll_interval = Duration::from_millis(ms.max(10));
    }
    let mut bridges: Vec<Arc<dyn EventBridge>> = Vec::new();
    if !a.has("no-bridge") {
        bridges.push(Arc::new(TimerBridge {
            agent: c.agent.clone(),
            who: c.who.clone(),
        }));
        if let Some(client) = &c.kevent {
            bridges.push(Arc::new(KEventBridge {
                client: client.clone(),
                agent: c.agent.clone(),
                who: c.who.clone(),
            }));
        }
    }
    // Commands run by `shell` reach the same queue and identity.
    if let Some(dir) = a.env_or("queue-dir", "LIBOPENDAN_QUEUE_DIR") {
        std::env::set_var("LIBOPENDAN_QUEUE_DIR", dir);
    }
    std::env::set_var("LIBOPENDAN_WHO", &c.who);
    Ok(HostDeps {
        who: c.who.clone(),
        agent: c.agent.clone(),
        inputs: c.channels.clone(),
        waker: c.waker.clone(),
        xllm: XllmDeps::default(),
        assembler: Arc::new(BehaviorAssembler::default()),
        session_cli: std::env::current_exe().ok(),
        app_tools: Vec::new(),
        options,
        max_child_concurrency: a.num::<usize>("max-children")?.unwrap_or(4),
        bridges,
        runtime_id: a.get("runtime"),
    })
}

/// Exit code of a drive result (§8).
fn exit_code(r: &DriveResult) -> i32 {
    match r {
        DriveResult::TurnClosed { status, .. } => match status {
            TurnStatus::Completed => 0,
            TurnStatus::Failed | TurnStatus::BudgetExhausted => 1,
            TurnStatus::Stopped => 4,
        },
        DriveResult::Finished { outcome, .. } => match outcome {
            Some(Outcome::Failed) => 1,
            Some(Outcome::Stopped) => 4,
            _ => 0,
        },
        DriveResult::Idle { .. }
        | DriveResult::TurnOpen { .. }
        | DriveResult::OutcomesHandled { .. } => 3,
        DriveResult::Busy { .. } | DriveResult::RunBusy { .. } | DriveResult::LeaseLost => 5,
        DriveResult::NotDriver { .. }
        | DriveResult::Unregistered
        | DriveResult::BindFailed { .. }
        | DriveResult::RecoveryBlocked(_) => 6,
        DriveResult::Error { error, .. } => {
            match error.get("kind").and_then(Value::as_str).unwrap_or_default() {
                "invalid_argument" | "channel" | "json" => 2,
                "session_readonly" | "runtime_mismatch" | "bind_failed" | "recovery_blocked" => 6,
                "input_full" => 3,
                _ => 1,
            }
        }
    }
}

/// SIGINT of a driving command is a stop of the session it drives: the run
/// is interrupted and the Turn closes as stopped, through the same path as
/// a queued `stop`.
fn stop_on_sigint() -> StopSignal {
    let stop = StopSignal::default();
    let signal = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("xagent: interrupted, stopping the session");
            signal.request();
        }
    });
    stop
}

async fn drive_and_report(c: &Ctx, a: &Args, sd: &SessionDir, created: bool) -> R<i32> {
    let until = parse_until(a)?;
    let h = host(c, a)?;
    let out = run_session(&h, sd, until, stop_on_sigint(), a.has("detach-children")).await;
    let mut v = serde_json::to_value(&out.result)?;
    if created {
        v["session_id"] = json!(sd.sid());
        v["path"] = json!(sd.path());
    }
    if !out.children.is_empty() {
        // Registered and committed; a resident host takes them over.
        v["children"] = json!(out.children);
    }
    if let DriveResult::RecoveryBlocked(b) = &out.result {
        eprintln!("xagent: recovery blocked: {b}");
    }
    print(&v);
    Ok(exit_code(&out.result))
}

async fn cmd_new(mut a: Args) -> R<i32> {
    let c = ctx(&a, None, true).await?;
    let class = a.get("class").unwrap_or_else(|| "work".to_string());
    let template = SessionTemplate::load(&class, c.agent.agent_root())?;
    let mut spec = template.spec(
        a.get("objective")
            .ok_or_else(|| bad("missing --objective"))?,
    );
    spec.idempotency_key = a.get("key");
    spec.prompt.llm_context = match a.get("llm-context") {
        None => json!({ "tools": { "enabled": true } }),
        Some(s) => serde_json::from_str(&file_or_inline(&s)?)?,
    };
    if let Some(b) = a.get("behavior") {
        spec.prompt.behavior = Some(b);
    }
    spec.runtime.requirement.runtime_id = a.get("runtime");
    if let Some(ws) = a.get("workspace") {
        spec.workspace = Some(WorkspaceRef::External { path: ws });
    }
    for s in a.all("subscribe") {
        spec.subscriptions.push(parse_subscription(&s)?);
    }
    template.settle_input(&mut spec);
    spec.freeze = true;
    let queued = spec.input_channel == Some(InputChannel::Queue);
    let first = agent_input(&c, &a)?;
    if !queued {
        // No queue: the first input is persisted with the configuration.
        spec.prompt.initial_inputs = first.iter().cloned().collect();
    }
    let parent = match (a.get("parent"), c.agent.agent_root()) {
        (Some(p), _) => PathBuf::from(p),
        (None, Some(root)) => root.join("sessions"),
        (None, None) => std::env::current_dir()?,
    };
    let sd = create_session(&parent, spec, c.agent.as_ref(), &c.who, c.channels.as_ref()).await?;
    if queued {
        if let Some(input) = &first {
            c.agent.sessions().post_input(sd.sid(), input).await?;
        }
    }
    if a.has("no-run") {
        print(&json!({ "session_id": sd.sid(), "path": sd.path(), "input_queue": queued }));
        return Ok(0);
    }
    // Not a session delivery any more: the input is in the session.
    a.opts.retain(|(n, _)| !matches!(n.as_str(), "msg" | "msg-file" | "event" | "json" | "runtime"));
    drive_and_report(&c, &a, &sd, true).await
}

async fn cmd_run(mut a: Args) -> R<i32> {
    let target = a.next("<session_dir|sid>")?;
    let (c, sd) = locate(&a, &target, true).await?;
    if let Some(input) = agent_input(&c, &a)? {
        if sd.config()?.channels.kmsg().is_none() {
            return Err(bad(format!(
                "session {} has no input queue: its inputs are given when it is created (`xagent new --msg`), or use a class with a queue",
                sd.sid()
            )));
        }
        c.agent.sessions().post_input(sd.sid(), &input).await?;
    }
    drive_and_report(&c, &a, &sd, false).await
}

async fn cmd_serve(mut a: Args) -> R<i32> {
    if a.pos.is_empty() {
        return Err(bad("missing <session_dir|sid>..."));
    }
    let first = a.pos.front().cloned().unwrap_or_default();
    let (c, sd) = locate(&a, &first, true).await?;
    let mut targets = vec![sd];
    a.pos.pop_front();
    while let Some(t) = a.pos.pop_front() {
        let sd = if Path::new(&t).join(STATE_DIR).is_dir() {
            SessionDir::open(&t)?
        } else {
            let e = c
                .agent
                .sessions()
                .lookup(&t)
                .await?
                .ok_or_else(|| bad(format!("session {t} is not registered")))?;
            SessionDir::open(&e.location)?
        };
        targets.push(sd);
    }
    let h = host(&c, &a)?;
    let idle = a.num::<u64>("idle-unload")?.map(Duration::from_secs);
    // SIGINT ends the hosting, not the sessions: their committed state
    // stays and another host takes them over.
    let served = tokio::select! {
        r = serve(&h, targets, StopSignal::default(), idle) => r,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("xagent: interrupted, hosting ends (sessions keep their committed state)");
            print(&json!({ "kind": "interrupted" }));
            return Ok(4);
        }
    };
    let code = served.iter().map(|(_, r)| exit_code(r)).max().unwrap_or(0);
    print(&served
        .iter()
        .map(|(sid, r)| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            v["session_id"] = json!(sid);
            v
        })
        .collect::<Vec<_>>());
    Ok(code)
}

fn env_sid() -> Option<String> {
    std::env::var("OPENDAN_SESSION_ID").ok().filter(|s| !s.is_empty())
}

async fn cmd_post(mut a: Args) -> R<i32> {
    let sid = a.next("<sid>")?;
    let c = ctx(&a, None, true).await?;
    let input = agent_input(&c, &a)?.ok_or_else(|| bad("post needs --msg or --json"))?;
    let idx = c.agent.sessions().post_input(&sid, &input).await.map_err(|e| match e {
        OpenDanError::Channel(m) if m.contains("no input queue") => bad(format!(
            "{m}: its inputs are given when it is created, or use a class with a queue"
        )),
        e => e.into(),
    })?;
    print(&json!({ "posted": idx, "key": input.key }));
    Ok(0)
}

const CTL_COMMANDS: &[&str] = &["stop", "decide", "subscribe", "unsubscribe", "activity", "perceive"];

async fn cmd_ctl(mut a: Args) -> R<i32> {
    let first = a.next("<sid> | <command>")?;
    let (sid, cmd) = if CTL_COMMANDS.contains(&first.as_str()) {
        (
            env_sid().ok_or_else(|| bad("missing <sid> (or $OPENDAN_SESSION_ID)"))?,
            first,
        )
    } else {
        (first, a.next("stop | decide | subscribe | unsubscribe | activity | perceive")?)
    };
    let c = ctx(&a, None, true).await?;
    let given_key = a.get("key");
    let key = |prefix: &str| {
        given_key
            .clone()
            .unwrap_or_else(|| format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
    };
    let (key, command) = match cmd.as_str() {
        "stop" => (key("stop"), ControlCommand::Stop { reason: a.get("reason") }),
        "decide" => {
            let decision = a.next("accept|discard")?;
            if !matches!(decision.as_str(), "accept" | "discard") {
                return Err(bad(format!("decide takes accept | discard, not `{decision}`")));
            }
            let entry = c
                .agent
                .sessions()
                .lookup(&sid)
                .await?
                .ok_or_else(|| bad(format!("session {sid} is not registered")))?;
            if entry.input_queue.is_none() {
                return decide_without_queue(&c, &entry, &decision).await;
            }
            (
                key("decide"),
                ControlCommand::Decide {
                    decision,
                    by: c.who.clone(),
                    note: a.get("note"),
                },
            )
        }
        "subscribe" => (
            key("subscribe"),
            ControlCommand::Subscribe {
                subscription: parse_subscription(&a.next("<spec>")?)?,
            },
        ),
        "unsubscribe" => (
            key("unsubscribe"),
            ControlCommand::Unsubscribe { id: a.next("<id>")? },
        ),
        "activity" => {
            let touch = a
                .all("touch")
                .into_iter()
                .map(|t| Touching {
                    kind: if t.starts_with("artifact:") { "artifact" } else { "path" }.into(),
                    target: t,
                    mode: "write".into(),
                    since_ms: libopendan::now_ms(),
                })
                .collect();
            (
                key("activity"),
                ControlCommand::Activity {
                    summary: a.get("summary"),
                    touch,
                    clear: a.has("clear"),
                },
            )
        }
        "perceive" => (
            key("perc"),
            ControlCommand::Perceive {
                kind: a.get("kind").unwrap_or_else(|| "observation".into()),
                summary: a.next("<text>")?,
                tags: a.all("tag"),
                objects: Vec::new(),
            },
        ),
        other => return Err(bad(format!("unknown control `{other}`"))),
    };
    let input = PostedInput::control(&c.who, key, command);
    let idx = c.agent.sessions().post_input(&sid, &input).await.map_err(|e| match e {
        OpenDanError::Channel(m) if m.contains("no input queue") => bad(format!(
            "{m}: without a queue only `decide` is available (stop = interrupt the process that drives it)"
        )),
        e => e.into(),
    })?;
    print(&json!({ "posted": idx, "key": input.key }));
    Ok(0)
}

/// A session without a queue has no driver to apply a `decide`: the
/// decision goes to the artifact directly, under its lock.
async fn decide_without_queue(c: &Ctx, entry: &RegistryEntry, decision: &str) -> R<i32> {
    let aid = entry.artifact_id.clone().ok_or_else(|| {
        bad(format!(
            "session {} has no input queue and no artifact: there is nothing a decision could change",
            entry.session_id
        ))
    })?;
    let holder = HolderInfo {
        runner_id: libopendan::ids::new_runner_id(),
        principal: c.who.clone(),
        host: Some(libopendan::runtime::native_host_id()),
        pid: std::process::id(),
        runtime_id: None,
    };
    let lease = match c.agent.locks().acquire(&format!("artifact:{aid}"), holder)? {
        libopendan::lock::Acquire::Acquired(l) => l,
        libopendan::lock::Acquire::Busy(_) => {
            return Err(Fail {
                code: 5,
                msg: format!("artifact {aid} is being decided by someone else"),
            })
        }
    };
    let ver = format!("v-{}", entry.session_id);
    let r = c.agent.artifacts().decide(&lease, &aid, &ver, decision).await;
    lease.release();
    let r = r?;
    print(&json!({ "artifact": aid, "version": r.version, "head": r.head_after }));
    Ok(0)
}

async fn cmd_status(mut a: Args) -> R<i32> {
    let target = a.next("<sid>")?;
    let (c, sd) = locate(&a, &target, false).await?;
    let n: usize = a.num("worklog")?.unwrap_or(0);
    let view = libopendan::read_session(c.agent.as_ref(), sd.sid(), false, a.has("report"), n).await?;
    let mut v = serde_json::to_value(&view)?;
    let state = sd.state()?;
    v["turn"] = json!({ "seq": state.turn_seq, "completed": state.turns_completed, "open": state.open_turn });
    v["holder"] = serde_json::to_value(sd.holder())?;
    if a.has("run") {
        let run_id = state.live_run.as_ref().map(|l| l.run_id.clone()).or(state.last_run.clone());
        v["run"] = match run_id {
            Some(id) => sd
                .runs()
                .record(&id)
                .ok()
                .map(|r| json!({
                    "run_id": r.run_id, "status": r.status, "handover": r.handover,
                    "host_commit_pending": r.host_commit_pending, "usage": r.usage,
                    "inflight": r.inflight.len(), "live": state.live_run.is_some(),
                }))
                .unwrap_or(Value::Null),
            None => Value::Null,
        };
    }
    if a.has("events") {
        let injected: Vec<Value> = sd
            .worklog()
            .recent(state.worklog.committed_bytes, 200)?
            .into_iter()
            .filter_map(|(_, e)| match e.body {
                WorklogBody::TurnStarted { events, input_seq, turn, .. }
                | WorklogBody::InputBatch { events, input_seq, turn, .. }
                    if !events.is_empty() =>
                {
                    Some(json!({ "turn": turn, "input_seq": input_seq, "events": events }))
                }
                _ => None,
            })
            .take(8)
            .collect();
        v["events"] = json!({ "pending": state.pending_events, "watched_tasks": state.watched_tasks, "recently_injected": injected });
    }
    print(&v);
    Ok(0)
}

async fn cmd_list(a: Args) -> R<i32> {
    let c = ctx(&a, None, false).await?;
    if a.has("active") {
        print(&c.agent.activity().active(None, 50).await?);
    } else {
        print(&c.agent.sessions().query(&RegistryQuery::default()).await?);
    }
    Ok(0)
}

async fn cmd_behaviors(a: Args) -> R<i32> {
    if let Some(target) = a.get("frozen") {
        let (_, sd) = locate(&a, &target, false).await?;
        let cfg = sd.config()?;
        print(&json!({
            "session_id": sd.sid(),
            "entry": cfg.prompt.behavior,
            "frozen": cfg.prompt.frozen,
            "entries": cfg.extensions.get("opendan").and_then(|o| o.get("behaviors")),
        }));
        return Ok(0);
    }
    let c = ctx(&a, None, false).await?;
    let catalog = c.agent.behaviors();
    print(&json!({ "revision": catalog.revision().await?, "behaviors": catalog.list().await? }));
    Ok(0)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn cmd_xllm(mut a: Args) -> R<i32> {
    let target = a.next("<sid>")?;
    let (_, sd) = locate(&a, &target, false).await?;
    let state = sd.state()?;
    let run_id = a
        .get("run")
        .or_else(|| state.live_run.as_ref().map(|l| l.run_id.clone()))
        .ok_or_else(|| bad(format!("session {} has no live run to hand to xllm", sd.sid())))?;
    let record = sd.runs().record(&run_id)?;
    let command = format!(
        "xllm --resume --run {} --runs-dir {} --dir {}",
        shell_quote(&run_id),
        shell_quote(&sd.runs_dir().display().to_string()),
        shell_quote(&record.workdir),
    );
    // Why xllm would refuse it, as far as it can be told from the record.
    let mut blockers = Vec::new();
    if record.host_commit_pending.is_some() {
        blockers.push("an input batch is not committed to the session yet (host_commit_pending)");
    }
    if record.handover.is_some() {
        blockers.push("the run stopped at a behavior hand-over; only the session commits the transfer");
    }
    if record.status.is_terminal() {
        blockers.push("the run is finished");
    }
    print(&json!({ "run_id": run_id, "status": record.status, "command": command, "blockers": blockers }));
    Ok(0)
}

fn cmd_schema(mut a: Args) -> R<i32> {
    let out = PathBuf::from(a.next("<dir>")?);
    std::fs::create_dir_all(&out)?;
    for (name, schema) in libopendan::protocol::json_schemas() {
        let p = out.join(format!("{name}.schema.json"));
        std::fs::write(&p, serde_json::to_vec_pretty(&schema)?)?;
        println!("{}", p.display());
    }
    Ok(0)
}

// ---- commands of a session's own shell (layer ② session tools) ----

async fn cmd_recall(a: Args) -> R<i32> {
    let c = ctx(&a, None, false).await?;
    let hints = c
        .agent
        .cognition()
        .recall_hints(&libopendan::state::RecallQuery {
            tags: a.pos.iter().cloned().collect(),
            max_hints: a.num("max")?.unwrap_or(5),
        })
        .await?;
    print(&hints);
    Ok(0)
}

async fn cmd_note(mut a: Args) -> R<i32> {
    let text = a.next("<text>")?;
    let c = ctx(&a, None, false).await?;
    let note = libopendan::state::NotebookNote {
        notebook_id: a.get("notebook").unwrap_or_else(|| "default".into()),
        title: a
            .get("title")
            .unwrap_or_else(|| text.lines().next().unwrap_or_default().chars().take(80).collect()),
        content: text,
        tags: a.all("tag"),
        session_id: env_sid(),
    };
    c.agent.cognition().notebook_append(&note, &c.who).await?;
    print(&json!({ "noted": note.title }));
    Ok(0)
}

fn brief(e: &RegistryEntry) -> Value {
    json!({
        "session_id": e.session_id, "class": e.class, "objective": e.objective,
        "run_state": e.status.run_state, "outcome": e.status.outcome,
        "acceptance": e.status.acceptance, "one_line_status": e.status.one_line_status,
        "waiting_for": e.status.waiting_for, "pending_decision": e.status.pending_decision,
        "parent": e.origin.as_ref().and_then(|o| o.parent_session.clone()),
    })
}

async fn cmd_sessions(a: Args) -> R<i32> {
    let c = ctx(&a, None, false).await?;
    let mut entries = if a.has("children") {
        let me = env_sid()
            .or_else(|| a.get("of"))
            .ok_or_else(|| bad("--children needs the calling session ($OPENDAN_SESSION_ID or --of <sid>)"))?;
        c.agent.sessions().children_of(&[me]).await?
    } else {
        c.agent.sessions().query(&RegistryQuery::default()).await?
    };
    if a.has("active") {
        entries.retain(|e| e.status.run_state != RunState::Finished);
    }
    print(&entries.iter().map(brief).collect::<Vec<_>>());
    Ok(0)
}

async fn cmd_read_session(mut a: Args) -> R<i32> {
    let sid = a.next("<sid>")?;
    let c = ctx(&a, None, false).await?;
    let n: usize = a.num("worklog")?.unwrap_or(0);
    // The agent's own side: `agent_access = status_only` is honoured.
    print(&libopendan::read_session(c.agent.as_ref(), &sid, true, a.has("report"), n).await?);
    Ok(0)
}

async fn cmd_artifact(mut a: Args) -> R<i32> {
    let sub = a.next("head <aid> | list")?;
    let c = ctx(&a, None, false).await?;
    match sub.as_str() {
        "head" => print(&c.agent.artifacts().head(&a.next("<aid>")?).await?),
        "list" => print(&c.agent.artifacts().list().await?),
        // Versions are registered by the session's driver when it finishes.
        other => return Err(bad(format!("artifact {other}: only `head <aid>` and `list` are available"))),
    }
    Ok(0)
}

/// A tool result the `shell` tool forwards as the call's own result:
/// `Pending` with the task id of the sub session being waited for.
fn pending_on_session(tool: &str, sid: &str, wait_ms: Option<u64>) -> AgentToolResult {
    let mut details = json!({ "session_id": sid, "status": "waiting" });
    if let Some(ms) = wait_ms {
        details["until_ms"] = json!(libopendan::now_ms() + ms);
    }
    let mut r = AgentToolResult::from_details(details)
        .with_tool(tool)
        .with_status(AgentToolStatus::Pending);
    r.task_id = Some(format!("{SESSION_TASK_PREFIX}{sid}"));
    r.summary = format!("waiting for sub session {sid}");
    r
}

async fn cmd_create_worksession(a: Args) -> R<i32> {
    let parent = env_sid()
        .or_else(|| a.get("parent-session"))
        .ok_or_else(|| bad("create-worksession runs inside a session ($OPENDAN_SESSION_ID)"))?;
    let c = ctx(&a, None, true).await?;
    // The creating call: the idempotency key of the sub session.
    let call = match (std::env::var(agent_tool::llm_bash::ENV_CALL_ID), a.get("key")) {
        (Ok(call_id), None) if !call_id.is_empty() => {
            let run = c
                .agent
                .sessions()
                .lookup(&parent)
                .await?
                .and_then(|e| SessionDir::open(&e.location).ok())
                .and_then(|sd| sd.state().ok())
                .and_then(|s| s.live_run.map(|l| l.run_id))
                .unwrap_or_default();
            Some((run, call_id))
        }
        _ => None,
    };
    let context_recent = match a.get("context").as_deref() {
        None | Some("none") => 0,
        Some(s) => s
            .strip_prefix("recent:")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| bad(format!("bad --context {s} (recent:<n> | none)")))?,
    };
    let report = match a.get("report").as_deref() {
        None | Some("final") => ReportMode::Final,
        Some("progress") => ReportMode::Progress,
        Some("none") => ReportMode::None,
        Some(s) => return Err(bad(format!("bad --report {s} (final | progress | none)"))),
    };
    let wait = a.has("wait");
    if wait && report == ReportMode::None {
        // Waiting is hearing the end.
        return Err(bad("--wait waits for the sub session's end; it cannot be combined with --report none"));
    }
    let sub = SubSessionSpec {
        objective: a.get("objective").ok_or_else(|| bad("missing --objective"))?,
        msgs: a.all("msg"),
        attachments: a
            .all("attach")
            .into_iter()
            .map(|att| match att.split_once('=') {
                Some((id, name)) => (id.to_string(), Some(name.to_string())),
                None => (att, None),
            })
            .collect(),
        context_recent,
        class: a.get("class"),
        behavior: a.get("behavior"),
        workspace: match a.get("workspace").as_deref() {
            None | Some("inherit") => SubWorkspace::Inherit,
            Some("new") => SubWorkspace::New,
            Some(id) => SubWorkspace::Id(id.to_string()),
        },
        runtime_id: a.get("runtime").filter(|r| r != "inherit"),
        report,
        interactive: a.has("interactive"),
        call,
        key: a.get("key"),
    };
    let sd = create_sub_session(c.agent.as_ref(), &c.who, &parent, sub, c.channels.as_ref()).await?;
    if wait {
        print(&pending_on_session("create-worksession", sd.sid(), a.num("wait-ms")?));
    } else {
        print(&json!({ "session_id": sd.sid(), "status": "created", "path": sd.path() }));
    }
    Ok(0)
}

async fn cmd_wait(mut a: Args) -> R<i32> {
    let sid = a.next("<sid>")?;
    let c = ctx(&a, None, false).await?;
    if c.agent.sessions().lookup(&sid).await?.is_none() {
        return Err(bad(format!("session {sid} is not registered")));
    }
    print(&pending_on_session("wait", &sid, a.num("wait-ms")?));
    Ok(0)
}

async fn real_main() -> R<i32> {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return Ok(if argv.is_empty() { 2 } else { 0 });
    }
    let cmd = argv.remove(0);
    // `--run` is a flag of `status` and takes a run id for `xllm`.
    let extra: &[&str] = if cmd == "status" { &["run"] } else { &[] };
    let a = Args::parse(argv, extra);
    if a.has("help") {
        println!("{USAGE}");
        return Ok(0);
    }
    match cmd.as_str() {
        "new" => cmd_new(a).await,
        "run" => cmd_run(a).await,
        "serve" => cmd_serve(a).await,
        "post" => cmd_post(a).await,
        "ctl" => cmd_ctl(a).await,
        "status" => cmd_status(a).await,
        "list" => cmd_list(a).await,
        "behaviors" => cmd_behaviors(a).await,
        "xllm" => cmd_xllm(a).await,
        "schema" => cmd_schema(a),
        "recall" => cmd_recall(a).await,
        "note" => cmd_note(a).await,
        "sessions" => cmd_sessions(a).await,
        "read-session" => cmd_read_session(a).await,
        "artifact" => cmd_artifact(a).await,
        "create-worksession" => cmd_create_worksession(a).await,
        "wait" => cmd_wait(a).await,
        other => {
            eprintln!("xagent: unknown command `{other}`\n\n{USAGE}");
            Ok(2)
        }
    }
}

#[tokio::main]
async fn main() {
    let code = match real_main().await {
        Ok(code) => code,
        Err(f) => {
            eprintln!("xagent: {}", f.msg);
            f.code
        }
    };
    std::process::exit(code);
}
