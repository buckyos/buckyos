//! xagent — drive an Agent Session for one Turn (or keep driving it).
//!
//! The layer above xllm: xllm advances one LLMContext run to an outcome,
//! xagent loads (or creates) an Agent Session and advances it until a Turn
//! closes, or serves it. Design: `doc/opendan/xAgent.md`.
//!
//! The same executable is wrapped as `agent-session` in every session's
//! `.runtime/bin`: commands run by `shell` use it to reach their session
//! and the Agent State.
//!
//! [`main`] is the entry point of the `xagent` binary and of the multi-call
//! `opendan` executable when it is invoked as `xagent` (aios image).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use agent_tool::{AgentToolResult, AgentToolStatus};
use buckyos_api::msg_queue::MsgQueueClient;
use crate::api::{create_session, create_sub_session, SubSessionSpec, SubWorkspace};
use crate::bridge::{EventBridge, KEventBridge, TimerBridge};
use crate::channel::{InputChannelFactory, InputSource, KEventWaker, KmsgChannels, PollWaker, Waker};
use crate::host::goals::{GoalHost, GoalHostOptions};
use crate::host::{run_session, serve, HostDeps, Supervisor};
use crate::protocol::*;
use crate::runner::{
    BehaviorAssembler, DriveResult, RunnerOptions, StopSignal, StopWhen, SESSION_TASK_PREFIX,
};
use crate::state::{
    connect, ActivationQuery, AgentStateClient, ConnectOptions, GoalCaller, GoalCreate,
    GoalReopen, GoalRestart, GoalRevise, GoalStatusChange, StateLocator,
};
use crate::{InputChannel, OpenDanError, SessionDir, SessionTemplate};
use serde_json::{json, Value};

const USAGE: &str = "\
xagent — drive an Agent Session for one Turn (or keep driving it)

  xagent new    --agent <did> --objective <text> [--class work|ui|self_check] [--parent <dir>]
                [--behavior <name>] [--cmd-skill <name>]... [--llm-context <json|@file>] [--runtime <id>] [--workspace <id>]
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
  xagent workspace list|get|create|import|discover|check|archive|restore|unregister|set-url|runtime-impact|set-runtime ...
        (list: [--active] [--runtime <id>] [--text <text>] [--json])
        (create/import: --runtime <id> --directory <abs path on that runtime> [--url <https://project>])
  xagent runtime list [--available] [--json] | get <id> | check <id> | baseline [--json] [--check [--overlay]]
                 | register <id> --kind container|host_native|remote_ssh --name <n> [--description <t>] [--exec <json>] [--root <dir>]...
                   [--baseline standard|extended]
                 | update <id> --expected-revision <n> [--name <n>] [--description <t>] [--root <dir>]...
                 | unregister <id> --expected-revision <n>
  xagent goal   create --objective <text> [--title <t>] [--lifecycle achieve|maintain] [--achieved-when <t>]
                       [--healthy-state <t>] [--distance <t>] [--constraint <t>]... [--skill <name>[=<ref>]]...
                       [--workspace <id>] [--path <p>]... [--object <o>]... [--every <secs>] [--min-interval <secs>]
                       [--start] [--behavior <b>] [--budget <json|@file>] [--key <idem>] [--json <request|@file>]
  xagent goal   list | show [<id>] [--state] [--activations <n>] [--changes <n>] [--revisions] [--epochs] [--schedule]
  xagent goal   revise <id> --expected-revision <n> [--objective ..] [--title ..] [--every ..] [--budget <json>]
                       [--json <patch|@file>] [--reason <t>]
  xagent goal   pause|resume|archive [<id>] [--reason <t>] | achieved [<id>] --evidence <t>
  xagent goal   reopen <id> --reason <t> [--fresh-state] | restart <id> --reason <t> [--carry-state]
  xagent goal   activate <id> [--reason <t>] | post <id> --msg <t>
  xagent goal   skills add <name>[=<ref>] [--note <t>] | skills remove <name>     (inside a goal session)
  xagent goal   serve [--llm-context <json|@file>] [--idle-unload <secs>]      (host the agent's goals and sessions)
  xagent schema <dir>

  <spec> = active|semi:object:<id>[#<event>] | semi:session:<sid>[:watch=f1,f2] | active|semi:timer:<name>

  inside a session's shell (as `agent-session`; the session comes from the environment):
  ctl activity ... | ctl perceive <text> | recall <tag>... | note <text> | sessions [--active] [--children] [--json]
  read-session <sid> [--report] [--worklog <n>] | artifact head <aid> | artifact list
  create-worksession --objective <t> [--msg <t>]... [--attach <obj_id>[=<name>]]... [--context recent:<n>|none]
        [--class <c>] [--behavior <b>] [--cmd-skill <name>]... [--workspace inherit|none|<id>] [--runtime inherit|<id>]
        [--report final|progress|none] [--interactive] [--wait [--wait-ms <ms>]]
        (--cmd-skill: up to 8 manuals from `skill list`, fixed for the session's life; none given: a work
         session inherits its parent work's, a conversation's or a goal's are not inherited)
  skill list [--kind cmd|goal] [--json] | skill install <x.pikg | dir> [--replace] | skill remove <name> [--kind cmd|goal]
  wait <sid> [--wait-ms <ms>] | post <sid> --msg <t>
  todo add <title> (--content <t> | --content - | --content-file <path>) [--accept <criterion>]... [--scope <t>] [--evidence <t>]... [--skill <cmd_skill>]...
           [--after <todo_id>]... [--batch <b>] [--runtime <id>] [--workspace inherit|none|<id>] [--behavior <b>]
           (a serial Todo: PLAN/DO's next_behavior = do runs it, in its own work session when it names another runtime or
            workspace; `todo dispatch` starts it as a work session of its own (default behavior work_simple), in order)
  todo explore <title> --content <t> [same options] [--class <c>]  (add + dispatch at once as an exploration sub session)
  todo dispatch <todo_id> [--behavior <b>] [--class <c>]  (keeps the Todo's mode: a serial Todo starts only when no other
           serial Todo runs, every result is reviewed and its --after Todos are accepted; an exploration starts at once;
           the behavior is kept for later attempts) | todo list [--json] | todo show <todo_id> [--report]
  todo review <todo_id> (--accept | --reject | --need-evidence) --reason <t> [--followup <todo_id>]...
  todo cancel <todo_id> --reason <t> | todo clean --reason <t> | todo retry <todo_id> --reason <t>
  ask-user <question> [--option <id>=<label>]... [--allow-text]   (ui sessions: the result is the user's next message)
  copy --from <end> --to <end> [--replace] | copy status|cancel <transfer_id> | copy list
        <end> = session:<path> | workspace:<path> | <runtime_id>:<absolute path>; one regular file between the session's
        runtimes, verified by hash and committed by rename; an existing target only with --replace
        (exit 0 committed, 1 failed, 4 cancelled)
  task state|wait|cancel <task_id> [--wait-ms <ms>]   (a shell command that continued as a task, in the run that started it;
        wait: default 25s, max 30 min — give the shell call a larger wait_ms; exit 0 running/finished, 1 failed, 2 unknown)
  runtime list [--available] [--json] | get <id> | check <id> | baseline [--json] | workspace ...
        (workspace list, runtime list and sessions: readable text by default; --json for structured output)
  goal ... (ui sessions: create / show / post / pause / resume / archive / reopen / restart / activate;
            goal sessions: show / skills add|remove / achieved --evidence / archive --reason)
  notify send --title <t> [--message <text>] [--level quiet|normal|timely|urgent] [--expires-in <dur>] [--key <k>]
  notify list [--open] [--any-source] [--session <sid>] [--goal <id>] | show <notify_id>
  notify answer <notify_id> (--option <value> | --text <t>) | ack <notify_id> | cancel <notify_id> [--reason <r>]
         (answer / ack: on the user's behalf, for a notification the user answered in this conversation;
          never one this session sent; exit 5 when it is closed already, 6 when it is not yours to answer)

  xagent todo   list <sid> [--json] | show <todo_id> --session <sid> [--report]   (a session's TodoList, read only)
  xagent notify list [--open] [--session <sid>] [--goal <id>] | show <id> | answer <id> (--option <v> | --text <t> | --dismiss)
              | ack <id> | dismiss <id> | cancel <id> [--reason <r>]     (the owner's side; --dev and tests)

identity and location (flags or environment):
  --session-context <path>  a session's .opendan/session.json (default: found above the current directory when
                       there is no $OPENDAN_SESSION_ID; on a node runtime, commands then go to AgentService)
  --agent <did>        [$OPENDAN_AGENT_DID]     --who <principal>  [$LIBOPENDAN_WHO]
  --agent-root <dir>   [$OPENDAN_AGENT_ROOT]    --state-url <url>  [$OPENDAN_AGENT_STATE_URL]
  --queue-dir <dir>    [$LIBOPENDAN_QUEUE_DIR]  development file queue; without it the zone's kmsg

exit codes: 0 completed | 1 failed | 2 bad arguments / configuration | 3 not finished (idle, open Turn, input_full, queue_missing)
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
            OpenDanError::InputFull { .. } | OpenDanError::QueueMissing { .. } => 3,
            OpenDanError::Busy { .. }
            | OpenDanError::RunBusy { .. }
            | OpenDanError::LeaseLost(_)
            | OpenDanError::GoalConflict(_)
            | OpenDanError::NotifyConflict(_) => 5,
            OpenDanError::GoalBudgetExhausted(_) => 3,
            OpenDanError::Forbidden(_) => 6,
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
    "start",
    "state",
    "revisions",
    "epochs",
    "schedule",
    "fresh-state",
    "carry-state",
    "allow-text",
    "available",
    "open",
    "any-source",
    "dismiss",
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

fn list_label(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
        .unwrap_or_default()
}

fn inline_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn list_heading(name: &str, count: usize) {
    println!("{name} ({count}):");
    if count == 0 {
        println!("(none)");
    }
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
    async fn open(&self, cfg: &SessionConfig) -> crate::Result<Vec<Arc<dyn InputSource>>> {
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
        a.env_or("agent-root", crate::state::ENV_AGENT_ROOT),
        a.env_or("state-url", crate::state::ENV_AGENT_STATE_URL),
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
        id: format!("sub-{}", crate::ids::h(&[spec])),
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
    let max_children = a.num::<usize>("max-children")?.unwrap_or(4);
    // A dispatch beyond what this host drives at once is refused, not queued.
    std::env::set_var(crate::todos::ENV_MAX_CHILDREN, max_children.to_string());
    Ok(HostDeps {
        who: c.who.clone(),
        agent: c.agent.clone(),
        inputs: c.channels.clone(),
        waker: c.waker.clone(),
        xllm: XllmDeps::default(),
        assembler: Arc::new(BehaviorAssembler::default()),
        session_cli: session_cli(),
        session_helpers: crate::runtime::bin_overlay::standard_helpers(
            std::env::current_exe().ok().as_deref().and_then(std::path::Path::parent),
        ),
        app_tools: Vec::new(),
        options,
        max_child_concurrency: max_children,
        bridges,
        outbound: None,
        objects: None,
        turn_tasks: None,
        notify: crate::notify::StateNotify::of(c.agent.clone()),
        share: None,
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
                "input_full" | "queue_missing" => 3,
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
    if template.kind == SessionKind::Goal {
        return Err(bad(
            "a goal session belongs to a goal: create the goal with `xagent goal create`; its host creates the session",
        ));
    }
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
    if let Some(names) = cmd_skills_arg(&a, "cmd-skill") {
        spec.prompt.cmd_skills = names;
    }
    spec.runtime.requirement.runtime_id = a.get("runtime");
    if let Some(ws) = a.get("workspace") {
        spec.workspace = Some(WorkspaceRef { workspace_id: ws, access: WorkspaceAccess::ReadWrite });
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

/// `agent-session task state|wait|cancel <task_id>`: the task tools of the
/// run this shell command belongs to, through its task bridge.
async fn cmd_task(mut a: Args) -> R<i32> {
    use agent_tool::task_bridge::{call, TaskBridgeRequest, TaskOp, ENV_TASK_ENDPOINT};
    let op = match a.next("state | wait | cancel")?.as_str() {
        "state" => TaskOp::State,
        "wait" => TaskOp::Wait,
        "cancel" => TaskOp::Cancel,
        other => return Err(bad(format!("task takes state | wait | cancel, not `{other}`"))),
    };
    let task_id = a.next("<task_id>")?;
    let wait_ms = a
        .get("wait-ms")
        .map(|v| v.parse::<u64>().map_err(|_| bad("--wait-ms takes milliseconds")))
        .transpose()?;
    let endpoint = std::env::var(ENV_TASK_ENDPOINT)
        .ok()
        .filter(|e| !e.is_empty())
        .ok_or_else(|| {
            bad(format!(
                "no task bridge here (${ENV_TASK_ENDPOINT}): run it through the shell tool of the run whose task it is"
            ))
        })?;
    let resp = call(&endpoint, &TaskBridgeRequest { op, task_id, wait_ms })
        .await
        .map_err(|msg| Fail { code: 1, msg })?;
    println!("{}", resp.text);
    Ok(resp.exit_code())
}

fn env_sid() -> Option<String> {
    std::env::var("OPENDAN_SESSION_ID").ok().filter(|s| !s.is_empty())
}

/// The tool call this command runs for: xllm's variable on the agent's own
/// machine, the Node Executor's operation call id on a node.
fn env_call_id() -> Option<String> {
    [agent_tool::llm_bash::ENV_CALL_ID, buckyos_api::ENV_EXECUTOR_CALL_ID]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|c| !c.is_empty()))
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
        "stop" => (key("stop"), ControlCommand::stop(a.get("reason"))),
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
                // No queue, no drive that would apply it: decided directly,
                // under the session's lock, by its driver.
                let acceptance = crate::api::decide_unqueued(
                    c.agent.as_ref(),
                    &c.who,
                    &sid,
                    &decision,
                    a.get("note").as_deref(),
                )
                .await?;
                print(&json!({ "session_id": sid, "acceptance": acceptance }));
                return Ok(0);
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
                    since_ms: crate::now_ms(),
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
                objects: a.all("object"),
                subjects: a.all("subject"),
                cites: a.all("cite"),
                source: a.get("source"),
                occurred_at: a.get("occurred-at"),
                suggested_kind: a.get("suggest"),
                memory_intent: if a.has("explicit") { Some("explicit".into()) } else { a.get("intent") },
                mentions: a.all("mention"),
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

async fn cmd_status(mut a: Args) -> R<i32> {
    let target = a.next("<sid>")?;
    let (c, sd) = locate(&a, &target, false).await?;
    let n: usize = a.num("worklog")?.unwrap_or(0);
    let view = crate::read_session(c.agent.as_ref(), sd.sid(), false, a.has("report"), n).await?;
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
    if sd.config()?.workspace_binding.is_some() {
        return Err(bad("managed workspace runs require `xagent run` for binding verification and writer coordination"));
    }
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
    for (name, schema) in crate::protocol::json_schemas() {
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
        .recall_hints(&crate::state::RecallQuery {
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
    let note = crate::state::NotebookNote {
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
    let mut v = json!({
        "session_id": e.session_id, "class": e.class, "objective": e.objective,
        "run_state": e.status.run_state, "outcome": e.status.outcome,
        "acceptance": e.status.acceptance, "one_line_status": e.status.one_line_status,
        "waiting_for": e.status.waiting_for, "pending_decision": e.status.pending_decision,
        "parent": e.origin.as_ref().and_then(|o| o.parent_session.clone()),
    });
    // A UI session's objective is its design intent: whose conversation it
    // is shows in its route key, what it is about in its topic.
    if let Some(route) = &e.route_key {
        v["route_key"] = json!(route);
    }
    if let Some(topic) = &e.status.topic {
        v["topic"] = json!(topic);
    }
    v
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
    if a.has("json") {
        print(&entries.iter().map(brief).collect::<Vec<_>>());
    } else {
        list_heading("Sessions", entries.len());
        for e in &entries {
            let mut status = vec![inline_text(&e.class), e.status.run_state.as_str().to_string()];
            if let Some(outcome) = e.status.outcome {
                status.push(list_label(outcome));
            }
            println!("- {} [{}]: {}", e.session_id, status.join(", "), inline_text(&e.objective));
            if !e.status.one_line_status.trim().is_empty() {
                println!("  Status: {}", inline_text(&e.status.one_line_status));
            }
            if let Some(topic) = &e.status.topic {
                println!("  Topic: {}", inline_text(&topic.title));
            }
            if let Some(waiting) = e.status.waiting_for {
                println!("  Waiting for: {}", list_label(waiting));
            }
            if e.status.acceptance != Acceptance::NotApplicable {
                println!("  Acceptance: {}", list_label(e.status.acceptance));
            }
            if e.status.pending_decision.is_some() {
                println!("  Decision needed: agent-session read-session {}", e.session_id);
            }
            if let Some(parent) = e.origin.as_ref().and_then(|o| o.parent_session.as_deref()) {
                println!("  Parent: {parent}");
            }
        }
    }
    Ok(0)
}

async fn cmd_read_session(mut a: Args) -> R<i32> {
    let sid = a.next("<sid>")?;
    let c = ctx(&a, None, false).await?;
    let n: usize = a.num("worklog")?.unwrap_or(0);
    // The agent's own side: `agent_access = status_only` is honoured.
    print(&crate::read_session(c.agent.as_ref(), &sid, true, a.has("report"), n).await?);
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
        details["until_ms"] = json!(crate::now_ms() + ms);
    }
    let mut r = AgentToolResult::from_details(details)
        .with_tool(tool)
        .with_status(AgentToolStatus::Pending);
    // The deadline the session's run waits until (whole seconds).
    r.check_after = wait_ms.map(|ms| ms.div_ceil(1000).max(1));
    r.task_id = Some(format!("{SESSION_TASK_PREFIX}{sid}"));
    r.summary = format!("waiting for sub session {sid}");
    r
}

/// `ask-user`: a question to the user. The result is `Pending` on
/// `userinput:<call_id>`; the session sends the question and fills the
/// call with the user's next message.
fn cmd_ask_user(mut a: Args) -> R<i32> {
    if env_sid().is_none() {
        return Err(bad("ask-user runs inside a session ($OPENDAN_SESSION_ID)"));
    }
    let call_id = env_call_id()
        .ok_or_else(|| bad("ask-user is a tool call of the session's run ($XLLM_CALL_ID); run it as a shell command of the run"))?;
    let question = a.next("the question")?;
    let options = a
        .all("option")
        .into_iter()
        .map(|o| match o.split_once('=') {
            Some((id, label)) => Ok(UserInputOption {
                id: id.trim().to_string(),
                label: label.trim().to_string(),
            }),
            None => Err(bad(format!("bad --option {o} (<id>=<label>)"))),
        })
        .collect::<R<Vec<_>>>()?;
    let request = UserInputRequest {
        question,
        input: UserInputSpec {
            kind: if options.is_empty() {
                UserInputKind::Text
            } else {
                UserInputKind::Choice
            },
            options,
            allow_text: a.has("allow-text"),
        },
    };
    request.validate().map_err(bad)?;
    let mut r = AgentToolResult::from_details(serde_json::to_value(&request)?)
        .with_tool("ask-user")
        .with_status(AgentToolStatus::Pending);
    r.task_id = Some(format!("{USER_INPUT_TASK_PREFIX}{call_id}"));
    r.summary = "waiting for the user's answer".into();
    print(&r);
    Ok(0)
}

async fn cmd_create_worksession(a: Args) -> R<i32> {
    let parent = env_sid()
        .or_else(|| a.get("parent-session"))
        .ok_or_else(|| bad("create-worksession runs inside a session ($OPENDAN_SESSION_ID)"))?;
    let c = ctx(&a, None, true).await?;
    // The creating call: the idempotency key of the sub session.
    let call = match (env_call_id(), a.get("key")) {
        (Some(call_id), None) => {
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
        cmd_skills: cmd_skills_arg(&a, "cmd-skill"),
        workspace: match a.get("workspace").as_deref() {
            None | Some("inherit") => SubWorkspace::Inherit,
            Some("none") => SubWorkspace::None,
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

/// `copy --from <end> --to <end> [--replace]` | `copy status|cancel <id>` |
/// `copy list`: one file between the session's runtimes (primary-runtime
/// TODO §4); `<end>` is `session:<path>`, `workspace:<path>` or
/// `<runtime_id>:<absolute path>`. Exit 0 committed, 1 failed, 4 cancelled.
async fn cmd_copy(mut a: Args) -> R<i32> {
    let sid = env_sid().ok_or_else(|| bad("copy runs inside a session ($OPENDAN_SESSION_ID)"))?;
    let c = ctx(&a, None, false).await?;
    let entry = c
        .agent
        .sessions()
        .lookup(&sid)
        .await?
        .ok_or_else(|| bad(format!("session {sid} is not registered")))?;
    let sd = SessionDir::open(&entry.location)?;
    let record = match a.pos.front().map(String::as_str) {
        Some("status") => {
            a.next("status")?;
            crate::transfer::status(&sd, &a.next("<transfer_id>")?)?
        }
        Some("cancel") => {
            a.next("cancel")?;
            crate::transfer::request_cancel(&sd, &a.next("<transfer_id>")?)?
        }
        Some("list") => {
            print(&crate::transfer::list(&sd)?);
            return Ok(0);
        }
        Some(other) => return Err(bad(format!("copy takes --from/--to, status, cancel or list, not `{other}`"))),
        None => {
            let end = |flag: &str| -> R<crate::transfer::Endpoint> {
                a.get(flag)
                    .ok_or_else(|| bad(format!("missing --{flag} <session|workspace|runtime_id>:<path>")))?
                    .parse()
                    .map_err(|e: OpenDanError| bad(e.to_string()))
            };
            let req = crate::transfer::CopyRequest {
                from: end("from")?,
                to: end("to")?,
                replace: a.has("replace"),
                call_id: env_call_id(),
            };
            crate::transfer::copy(c.agent.as_ref(), &sd, &req).await?
        }
    };
    print(&record);
    Ok(record.exit_code())
}

/// `--cmd-skill <name>` (repeatable, or `a,b`). `None`: not given.
fn cmd_skills_arg(a: &Args, flag: &str) -> Option<Vec<String>> {
    let names: Vec<String> = a
        .all(flag)
        .iter()
        .flat_map(|v| v.split(',').map(|x| x.trim().to_string()))
        .filter(|x| !x.is_empty())
        .collect();
    (!names.is_empty()).then_some(names)
}

// ---------------------------------------------------------------------------
// skill — the agent's skills (cmd_skills / goal_skills)
// ---------------------------------------------------------------------------

fn skill_kind(a: &Args) -> R<Option<SkillKind>> {
    match a.get("kind").as_deref() {
        None => Ok(None),
        Some("cmd") => Ok(Some(SkillKind::Cmd)),
        Some("goal") => Ok(Some(SkillKind::Goal)),
        Some(k) => Err(bad(format!("bad --kind {k} (cmd | goal)"))),
    }
}

/// `skill list [--kind cmd|goal] [--json]` / `skill install <x.pikg | dir> [--replace]`
/// / `skill remove <name> [--kind cmd|goal]`: the agent's skills. A new
/// session takes the manuals installed when it is created; running sessions
/// keep theirs.
async fn cmd_skill(mut a: Args) -> R<i32> {
    let sub = a.next("list | install | remove")?;
    let c = ctx(&a, None, false).await?;
    match sub.as_str() {
        "list" => {
            let skills = c.agent.behaviors().skills(skill_kind(&a)?).await?;
            if a.has("json") {
                print(&skills);
                return Ok(0);
            }
            if skills.is_empty() {
                println!("no skills installed");
            }
            for s in &skills {
                let mut line = format!("{}  [{}] {}", s.name, s.kind.as_str(), s.description);
                if s.baseline != buckyos_api::RuntimeBaseline::Standard || !s.platforms.is_empty() {
                    line.push_str(&format!(" — needs: {} environment", s.baseline.as_str()));
                    if !s.platforms.is_empty() {
                        line.push_str(&format!(" on {}", s.platforms.join(", ")));
                    }
                }
                if !s.requires.is_empty() {
                    line.push_str(&format!(" — requires: {}", s.requires));
                }
                if !s.tools.is_empty() {
                    line.push_str(&format!(" — commands: {}", s.tools.join(", ")));
                }
                if let Some(i) = &s.install {
                    line.push_str(&format!(" — from {} {}", i.package, i.version));
                }
                println!("{line}");
            }
            Ok(0)
        }
        "install" => {
            let path = PathBuf::from(a.next("<skill package: .pikg file or directory>")?);
            let pkg = crate::skills::read_source(&path)?;
            let r = c.agent.behaviors().install_skills(&pkg, a.has("replace"), &c.who).await?;
            print(&r);
            Ok(0)
        }
        "remove" => {
            let name = a.next("<name>")?;
            let kind = skill_kind(&a)?.unwrap_or(SkillKind::Cmd);
            let m = c.agent.behaviors().remove_skill(kind, &name, &c.who).await?;
            print(&json!({ "removed": m }));
            Ok(0)
        }
        other => Err(bad(format!("unknown skill command `{other}` (list | install | remove)"))),
    }
}

// ---------------------------------------------------------------------------
// todo — the session's TodoList (doc/opendan/Agent TodoList.md)
// ---------------------------------------------------------------------------

fn todo_fail(e: agent_tool::AgentToolError) -> Fail {
    crate::todos::todo_error(e).into()
}

/// The Todo body: `--content <text>`, `--content -` (stdin: a quoted
/// heredoc keeps backticks and `$` as they are) or `--content-file <path>`.
fn todo_content(a: &Args) -> R<String> {
    match (a.get("content"), a.get("content-file")) {
        (Some(c), None) if c == "-" => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            Ok(text)
        }
        (Some(c), None) => Ok(c),
        (None, Some(path)) => Ok(std::fs::read_to_string(&path)
            .map_err(|e| bad(format!("--content-file {path}: {e}")))?),
        (Some(_), Some(_)) => Err(bad("use one of --content and --content-file")),
        (None, None) => Err(bad(
            "missing --content <what to do, in which scope, with what evidence> (or --content - with the body on stdin)",
        )),
    }
}

fn new_todo(a: &mut Args, mode: crate::todos::TodoMode) -> R<crate::todos::NewTodo> {
    Ok(crate::todos::NewTodo {
        title: a.next("<title>")?,
        content: todo_content(a)?,
        mode,
        accept: a.all("accept"),
        scope: a.get("scope"),
        evidence: a.all("evidence"),
        skills: a
            .all("skill")
            .iter()
            .flat_map(|v| v.split(',').map(|x| x.trim().to_string()))
            .filter(|x| !x.is_empty())
            .collect(),
        after: a
            .all("after")
            .iter()
            .flat_map(|v| v.split(',').map(|x| x.trim().to_string()))
            .filter(|x| !x.is_empty())
            .collect(),
        batch: a.get("batch"),
        runtime_id: a.get("runtime"),
        workspace: a.get("workspace"),
        behavior: a.get("behavior"),
    })
}

/// The Todo executor the session's current context hands serial Todos
/// over to (`next_behavior`): a behavior loop with an `[entry] todo`
/// behavior. `None`: Todos are dispatched as sub sessions of their own and
/// waited for with `agent-session wait` (function call loop).
fn todo_executor(sd: &SessionDir) -> Option<String> {
    let cfg = sd.config().ok()?;
    let current = sd
        .state()
        .ok()
        .and_then(|s| s.current_behavior)
        .or_else(|| cfg.prompt.behavior.clone())?;
    let behaviors = cfg.behaviors().ok()?;
    let loop_model = behaviors
        .get(&current)
        .and_then(|e| e.llm_context.get("loop_model").cloned())
        .or_else(|| cfg.prompt.llm_context.get("loop_model").cloned());
    if loop_model.as_ref().and_then(Value::as_str) != Some("behavior") {
        return None;
    }
    behaviors.into_iter().find(|(_, e)| e.todo).map(|(name, _)| name)
}

/// A Todo's `--skill` names are cmd_skills the agent has: the sub session
/// that executes it takes them.
async fn check_todo_skills(c: &Ctx, names: &[String]) -> R<()> {
    if names.is_empty() {
        return Ok(());
    }
    crate::state::check_cmd_skill_names(names)?;
    let installed: Vec<String> = c
        .agent
        .behaviors()
        .skills(Some(SkillKind::Cmd))
        .await?
        .into_iter()
        .map(|m| m.name)
        .collect();
    for n in names {
        if !installed.contains(n) {
            return Err(bad(format!(
                "--skill {n}: the agent has no such cmd_skill (installed: {}; see `agent-session skill list`)",
                installed.join(", ")
            )));
        }
    }
    Ok(())
}

fn explore_options(a: &Args) -> crate::todos::ExploreOptions {
    let mut o = crate::todos::ExploreOptions::from_env();
    o.behavior = a.get("behavior");
    o.class = a.get("class");
    o
}

/// Dispatch with an observable result: a refusal leaves the Todo pending
/// and says why (exit 3, the caller may wait and dispatch again).
/// A serial Todo keeps its mode: it starts in order as a work session of
/// its own (`dispatch_serial`); an exploration starts as one at once.
async fn todo_dispatch(c: &Ctx, sd: &SessionDir, todo_id: &str, a: &Args) -> R<i32> {
    use crate::todos::{store_of, TodoMode};
    let mode = store_of(sd)
        .load()
        .map_err(todo_fail)?
        .get(todo_id)
        .map(|t| t.mode)
        .ok_or_else(|| bad(format!("todo {todo_id}: no such todo")))?;
    let opts = explore_options(a);
    let dispatched = match mode {
        TodoMode::Serial => {
            crate::todos::dispatch_serial(c.agent.as_ref(), &c.who, c.channels.as_ref(), sd, todo_id, &opts).await
        }
        TodoMode::Explore => {
            crate::todos::dispatch(c.agent.as_ref(), &c.who, c.channels.as_ref(), sd, todo_id, &opts).await
        }
    };
    match dispatched {
        Ok(d) => {
            let note = match todo_executor(sd) {
                Some(_) => "the sub session reports back when it finishes; end your step with a progress report to wait for it".to_string(),
                None => format!(
                    "wait for it with `agent-session wait {}` as a shell call of its own, then read its report with `agent-session todo show {} --report` and review it",
                    d.child_sid, d.todo_id
                ),
            };
            print(&json!({
                "todo_id": d.todo_id, "attempt": d.attempt, "child_sid": d.child_sid,
                "mode": mode.as_str(), "status": "running", "dispatched": true, "note": note,
            }));
            Ok(0)
        }
        Err(OpenDanError::InvalidArgument(reason)) => {
            print(&json!({ "todo_id": todo_id, "status": "pending", "dispatched": false, "reason": reason }));
            Ok(3)
        }
        Err(e) => Err(e.into()),
    }
}

const TODO_COMMANDS: &str = "add | explore | dispatch | list | show | review | cancel | clean | retry";

/// `todo …`. Inside a session (`agent-session todo …`, $OPENDAN_SESSION_ID)
/// every command works on that session's list — the session's tool layer is
/// the list's writer. From outside only `list` and `show` read a list.
async fn cmd_todo(mut a: Args) -> R<i32> {
    use crate::todos::{store_of, ReviewVerdict, TodoMode};
    let sub = a.next(TODO_COMMANDS)?;
    let inside = env_sid();
    let target = match (sub.as_str(), a.get("session")) {
        (_, Some(sid)) => sid,
        ("list", None) if inside.is_none() => a.next("<sid>")?,
        (_, None) => inside
            .clone()
            .ok_or_else(|| bad("todo runs inside a session ($OPENDAN_SESSION_ID), or reads one with --session <sid>"))?,
    };
    let writes = !matches!(sub.as_str(), "list" | "show");
    if writes && inside.as_deref() != Some(target.as_str()) {
        return Err(bad(format!(
            "todo {sub} changes a session's list: only its own runs can (inside session {target})"
        )));
    }
    let (c, sd) = locate(&a, &target, sub == "explore" || sub == "dispatch" || sub == "retry").await?;
    let store = store_of(&sd);
    let now = crate::now_ms();
    match sub.as_str() {
        "add" => {
            let new = new_todo(&mut a, TodoMode::Serial)?;
            check_todo_skills(&c, &new.skills).await?;
            let id = store.update(|l| l.add(new, now)).map_err(todo_fail)?;
            let list = store.load().map_err(todo_fail)?;
            let note = match todo_executor(&sd) {
                Some(executor) => format!("next_behavior = {executor} executes the next ready serial todo"),
                None => format!(
                    "start it with `agent-session todo dispatch {id}` once it is ready: a work session of its own; serial todos run one at a time, after the todos they run after are accepted"
                ),
            };
            print(&json!({ "todo_id": id, "status": "pending", "mode": "serial", "rev": list.rev, "note": note }));
            Ok(0)
        }
        "explore" => {
            let new = new_todo(&mut a, TodoMode::Explore)?;
            check_todo_skills(&c, &new.skills).await?;
            let id = store.update(|l| l.add(new, now)).map_err(todo_fail)?;
            todo_dispatch(&c, &sd, &id, &a).await
        }
        "dispatch" => {
            let id = a.next("<todo_id>")?;
            todo_dispatch(&c, &sd, &id, &a).await
        }
        "list" => {
            let list = store.load().map_err(todo_fail)?;
            if a.has("json") {
                print(&list);
            } else {
                println!("{}", list.render_view());
            }
            Ok(0)
        }
        "show" => {
            let id = a.next("<todo_id>")?;
            let list = store.load().map_err(todo_fail)?;
            let t = list.get(&id).ok_or_else(|| bad(format!("todo {id}: no such todo")))?;
            let mut v = json!({ "todo": t, "review": t.review_state() });
            if a.has("report") {
                v["report"] = match t.latest() {
                    Some(attempt) => serde_json::to_value(
                        crate::todos::read_report(c.agent.as_ref(), &sd, attempt).await?,
                    )?,
                    None => Value::Null,
                };
            }
            print(&v);
            Ok(0)
        }
        "review" => {
            let id = a.next("<todo_id>")?;
            let verdict = match (a.has("accept"), a.has("reject"), a.has("need-evidence")) {
                (true, false, false) => ReviewVerdict::Accept,
                (false, true, false) => ReviewVerdict::Reject,
                (false, false, true) => ReviewVerdict::NeedEvidence,
                _ => return Err(bad("todo review takes exactly one of --accept | --reject | --need-evidence")),
            };
            let reason = a.get("reason").ok_or_else(|| bad("missing --reason <what the report shows and why>"))?;
            let (attempt, recorded) = store
                .update(|l| l.review(&id, verdict, &reason, a.all("followup"), Some(c.who.clone()), now))
                .map_err(todo_fail)?;
            let list = store.load().map_err(todo_fail)?;
            let child = list
                .get(&id)
                .and_then(|t| t.attempt(attempt))
                .and_then(|x| x.child_sid.clone());
            // An exploration's acceptance is its sub session's too.
            let mut out = json!({ "todo_id": id, "attempt": attempt, "verdict": verdict, "recorded": recorded });
            if let Some(child) = child {
                let decision = match verdict {
                    ReviewVerdict::Accept => Some("accept"),
                    ReviewVerdict::Reject => Some("discard"),
                    ReviewVerdict::NeedEvidence => None,
                };
                if let Some(decision) = decision {
                    match crate::api::decide_unqueued(c.agent.as_ref(), &c.who, &child, decision, Some(&reason)).await {
                        Ok(acc) => out["acceptance"] = json!({ "session_id": child, "acceptance": acc }),
                        Err(e) => {
                            out["acceptance_error"] = json!(format!(
                                "the review is recorded; the acceptance of {child} was not written ({e}): run the same review again"
                            ));
                            print(&out);
                            return Ok(5);
                        }
                    }
                }
            }
            print(&out);
            Ok(0)
        }
        "cancel" => {
            let id = a.next("<todo_id>")?;
            let reason = a.get("reason").ok_or_else(|| bad("missing --reason"))?;
            store.update(|l| l.cancel(&id, &reason, now)).map_err(todo_fail)?;
            print(&json!({ "todo_id": id, "status": "cancelled" }));
            Ok(0)
        }
        "clean" => {
            let reason = a.get("reason").ok_or_else(|| bad("missing --reason"))?;
            let ids = store.update(|l| l.clean(&reason, now)).map_err(todo_fail)?;
            print(&json!({ "cancelled": ids }));
            Ok(0)
        }
        "retry" => {
            let id = a.next("<todo_id>")?;
            let reason = a.get("reason").ok_or_else(|| bad("missing --reason <what the new attempt must do differently>"))?;
            store.update(|l| l.retry(&id, &reason, now)).map_err(todo_fail)?;
            let mode = store.load().map_err(todo_fail)?.get(&id).map(|t| t.mode);
            if mode == Some(TodoMode::Explore) {
                return todo_dispatch(&c, &sd, &id, &a).await;
            }
            let note = match todo_executor(&sd) {
                Some(executor) => format!("next_behavior = {executor} runs it again as a new attempt"),
                None => format!("start the new attempt with `agent-session todo dispatch {id}`"),
            };
            print(&json!({ "todo_id": id, "status": "pending", "note": note }));
            Ok(0)
        }
        other => Err(bad(format!("unknown todo command `{other}` ({TODO_COMMANDS})"))),
    }
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

async fn cmd_workspace(mut a: Args) -> R<i32> {
    let command = a.next("workspace command")?;
    let c = ctx(&a, None, false).await?;
    let manager = c.agent.workspaces();
    let required = |name: &str| a.get(name).ok_or_else(|| bad(format!("missing --{name}")));
    let location = || -> R<WorkspaceLocation> {
        let directory = PathBuf::from(required("directory")?);
        if !directory.is_absolute() {
            return Err(bad("--directory must be absolute in the selected Runtime"));
        }
        Ok(WorkspaceLocation {
            runtime_id: a
                .get("runtime")
                .unwrap_or_else(|| CONTAINER_RUNTIME.into()),
            directory,
        })
    };
    let usage = || -> R<WorkspaceUsage> {
        match a.get("usage").as_deref() {
            None | Some("collaborative") => Ok(WorkspaceUsage::Collaborative),
            Some("private") => Ok(WorkspaceUsage::Private),
            _ => Err(bad("--usage takes collaborative | private")),
        }
    };
    match command.as_str() {
        "list" => {
            let entries = manager
                .query(&WorkspaceQuery {
                    runtime_id: a.get("runtime"),
                    lifecycle: a.has("active").then_some(WorkspaceLifecycle::Active),
                    text: a.get("text"),
                    ..Default::default()
                })
                .await?;
            if a.has("json") {
                print(&entries);
            } else {
                list_heading("Workspaces", entries.len());
                for w in &entries {
                    println!(
                        "- {}: {} — {}:{} [{}, {}, {}]",
                        w.workspace_id, inline_text(&w.name), w.location.runtime_id,
                        w.location.directory.display(), list_label(w.lifecycle),
                        list_label(w.availability), list_label(w.usage),
                    );
                    if let Some(url) = &w.url {
                        println!("  URL: {url}");
                    }
                    if let Some(error) = &w.last_error {
                        println!("  Error: {}", inline_text(error));
                    }
                }
            }
        }
        "create" => print(
            &manager
                .create(
                    &WorkspaceCreate {
                        operation_id: required("key")?,
                        name: required("name")?,
                        description: a.get("description").unwrap_or_default(),
                        url: a.get("url"),
                        location: location()?,
                        usage: usage()?,
                        source_session: env_sid(),
                        policy_ref: a.get("policy"),
                    },
                    &c.who,
                )
                .await?,
        ),
        "import" => print(
            &manager
                .import(
                    &WorkspaceImport {
                        operation_id: required("key")?,
                        location: location()?,
                        name: a.get("name"),
                        description: a.get("description").unwrap_or_default(),
                        url: a.get("url"),
                        usage: usage()?,
                        expected_revision: a.num("expected-revision")?,
                        source_session: env_sid(),
                        policy_ref: a.get("policy"),
                    },
                    &c.who,
                )
                .await?,
        ),
        "discover" => print(
            &manager
                .discover(
                    &WorkspaceDiscover {
                        expected_workspace_id: a.get("id"),
                        location: location()?,
                        expected_revision: a.num("expected-revision")?,
                    },
                    &c.who,
                )
                .await?,
        ),
        "set-url" => {
            let id = a.next("workspace id")?;
            let revision = a.num("expected-revision")?.ok_or_else(|| {
                bad("missing --expected-revision (read workspace get first)")
            })?;
            print(
                &manager
                    .update(
                        &id,
                        &WorkspaceUpdate {
                            expected_revision: revision,
                            lifecycle: None,
                            private_notes: None,
                            url: Some(a.get("url").unwrap_or_default()),
                        },
                        &c.who,
                    )
                    .await?,
            );
        }
        "get" | "check" | "archive" | "restore" | "unregister" => {
            let id = a.next("workspace id")?;
            match command.as_str() {
                "get" => print(
                    &manager
                        .lookup(&id)
                        .await?
                        .ok_or_else(|| bad(format!("workspace {id} is not registered")))?,
                ),
                "check" => print(&manager.check(&id).await?),
                _ => {
                    let revision = a.num("expected-revision")?.ok_or_else(|| {
                        bad("missing --expected-revision (read workspace get first)")
                    })?;
                    if command == "unregister" {
                        manager.unregister(&id, revision, &c.who).await?;
                        print(&json!({"workspace_id": id, "unregistered": true}));
                    } else {
                        print(
                            &manager
                                .update(
                                    &id,
                                    &WorkspaceUpdate {
                                        expected_revision: revision,
                                        lifecycle: Some(if command == "archive" {
                                            WorkspaceLifecycle::Archived
                                        } else {
                                            WorkspaceLifecycle::Active
                                        }),
                                        private_notes: None,
                                        url: None,
                                    },
                                    &c.who,
                                )
                                .await?,
                        );
                    }
                }
            }
        }
        "runtime-impact" => print(&manager.runtime_impact(&a.next("runtime id")?).await?),
        "set-runtime" => {
            let runtime = a.next("runtime id")?;
            let available = match a.next("available | unavailable")?.as_str() {
                "available" => true,
                "unavailable" => false,
                _ => return Err(bad("set-runtime takes available | unavailable")),
            };
            print(
                &manager
                    .set_runtime_available(&runtime, available, &c.who)
                    .await?,
            );
        }
        _ => return Err(bad(format!("unknown workspace command `{command}`"))),
    }
    Ok(0)
}

/// `runtime baseline [--json] [--check [--overlay]]`: the standard
/// environment's commands; `--check` looks for them on this machine's PATH
/// (`--overlay`: session helpers come from a session bin overlay, so their
/// providing executables are looked for instead). Exit 1 when any is
/// missing.
fn cmd_runtime_baseline(a: &Args) -> R<i32> {
    use crate::baseline::{missing_locally, required_commands, STANDARD_TOOLS};
    if a.has("check") {
        let names = required_commands(a.has("overlay"));
        let missing = missing_locally(&names, None, &[]);
        print(&json!({ "baseline": "standard", "checked": names, "missing": missing, "ok": missing.is_empty() }));
        return Ok(if missing.is_empty() { 0 } else { 1 });
    }
    if a.has("json") {
        print(&STANDARD_TOOLS);
    } else {
        for t in STANDARD_TOOLS {
            println!("{:<22} {:<16} {}", t.name, serde_json::to_value(t.category).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(), t.source);
        }
    }
    Ok(0)
}

async fn cmd_runtime(mut a: Args) -> R<i32> {
    let command = a.next("runtime command")?;
    if command == "baseline" {
        return cmd_runtime_baseline(&a);
    }
    let c = ctx(&a, None, false).await?;
    let registry = c.agent.runtimes();
    match command.as_str() {
        "list" => {
            let mut all = registry.list().await?;
            if a.has("available") {
                all.retain(|r| r.availability == RuntimeAvailability::Available);
            }
            if a.has("json") {
                print(&all);
            } else {
                list_heading("Runtimes", all.len());
                for r in &all {
                    let platform = [&r.platform.os, &r.platform.arch, &r.platform.shell]
                        .into_iter()
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| inline_text(s))
                        .collect::<Vec<_>>();
                    println!(
                        "- {}: {} — {}; {}; {}{}; computer: {}",
                        r.runtime_id, inline_text(&r.name),
                        if platform.is_empty() { "platform unknown".into() } else { platform.join(" / ") },
                        r.baseline.as_str(), list_label(r.availability),
                        if r.default { "; default" } else { "" }, r.computer.as_str(),
                    );
                    if !r.roots.is_empty() {
                        println!("  Workspace roots: {}", r.roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
                    }
                    match &r.baseline_check {
                        Some(check) => {
                            if !check.missing.is_empty() {
                                println!("  Tool check: missing {}", check.missing.join(", "));
                            } else {
                                println!("  Tool check: {}", if check.ok { "ok" } else { "failed" });
                            }
                            if let Some(error) = &check.error {
                                println!("  Tool check error: {}", inline_text(error));
                            }
                        }
                        None => println!("  Tool check: not checked"),
                    }
                    if let Some(error) = &r.last_error {
                        println!("  Error: {}", inline_text(error));
                    }
                }
            }
        }
        "get" => {
            let id = a.next("runtime id")?;
            print(
                &registry
                    .lookup(&id)
                    .await?
                    .ok_or_else(|| bad(format!("runtime {id} is not registered")))?,
            );
        }
        "check" => print(&registry.check(&a.next("runtime id")?).await?),
        "register" => {
            let kind = match a.get("kind").as_deref() {
                Some("container") => RuntimeKind::Container,
                Some("host_native") => RuntimeKind::HostNative,
                Some("remote_node") => RuntimeKind::RemoteNode,
                Some("remote_ssh") => RuntimeKind::RemoteSsh,
                _ => return Err(bad("--kind takes container | host_native | remote_node | remote_ssh")),
            };
            let exec = a.get("exec").map(|e| json_arg::<Value>(&e)).transpose()?;
            print(
                &registry
                    .register(
                        &RuntimeRegister {
                            runtime_id: a.next("runtime id")?,
                            kind,
                            name: a.get("name").ok_or_else(|| bad("missing --name"))?,
                            description: a.get("description").unwrap_or_default(),
                            exec,
                            roots: a.all("root").into_iter().map(PathBuf::from).collect(),
                            capabilities: json!({}),
                            default: false,
                            computer: Default::default(),
                            baseline: match a.get("baseline").as_deref() {
                                None | Some("standard") => buckyos_api::RuntimeBaseline::Standard,
                                Some("extended") => buckyos_api::RuntimeBaseline::Extended,
                                Some(b) => return Err(bad(format!("--baseline takes standard | extended, not `{b}`"))),
                            },
                        },
                        &c.who,
                    )
                    .await?,
            );
        }
        "update" => {
            let id = a.next("runtime id")?;
            let roots = a.all("root");
            print(
                &registry
                    .update(
                        &id,
                        &RuntimeUpdate {
                            expected_revision: a.num("expected-revision")?.ok_or_else(|| {
                                bad("missing --expected-revision (read runtime get first)")
                            })?,
                            name: a.get("name"),
                            description: a.get("description"),
                            roots: (!roots.is_empty())
                                .then(|| roots.into_iter().map(PathBuf::from).collect()),
                        },
                        &c.who,
                    )
                    .await?,
            );
        }
        "unregister" => {
            let id = a.next("runtime id")?;
            let revision = a
                .num("expected-revision")?
                .ok_or_else(|| bad("missing --expected-revision (read runtime get first)"))?;
            registry.unregister(&id, revision, &c.who).await?;
            print(&json!({ "runtime_id": id, "unregistered": true }));
        }
        _ => return Err(bad(format!("unknown runtime command `{command}`"))),
    }
    Ok(0)
}

/// The caller of a goal operation: inside a session its kind decides what it
/// may do (`$OPENDAN_SESSION_ID`), outside the owner side.
fn goal_caller(c: &Ctx) -> GoalCaller {
    GoalCaller {
        principal: c.who.clone(),
        session: env_sid(),
    }
}

/// `<id>`, or the goal of the calling goal session.
async fn goal_id_arg(c: &Ctx, a: &mut Args) -> R<String> {
    if let Some(id) = a.pos.pop_front() {
        return Ok(id);
    }
    if let Some(sid) = env_sid() {
        if let Some(e) = c.agent.sessions().lookup(&sid).await? {
            if e.kind == SessionKind::Goal {
                if let Some(g) = e.goal_id {
                    return Ok(g);
                }
            }
        }
    }
    Err(bad("missing <goal_id>"))
}

fn json_arg<T: serde::de::DeserializeOwned>(s: &str) -> R<T> {
    Ok(serde_json::from_str(&file_or_inline(s)?)?)
}

fn skills_arg(a: &Args) -> Vec<GoalSkill> {
    a.all("skill")
        .into_iter()
        .map(|s| match s.split_once('=') {
            Some((n, r)) => GoalSkill {
                name: n.to_string(),
                reference: Some(r.to_string()),
                version: None,
                note: None,
            },
            None => GoalSkill {
                name: s,
                reference: None,
                version: None,
                note: None,
            },
        })
        .collect()
}

async fn goal_show(c: &Ctx, a: &Args, id: &str) -> R<Value> {
    let goals = c.agent.goals();
    let goal = goals
        .get(id)
        .await?
        .ok_or_else(|| bad(format!("goal {id} does not exist")))?;
    let mut v = json!({ "goal": goal });
    let session = c.agent.sessions().lookup(&goal.session.sid).await?;
    v["session"] = json!(session.map(|e| json!({
        "session_id": e.session_id, "run_state": e.status.run_state, "outcome": e.status.outcome,
        "waiting_for": e.status.waiting_for, "turn_open": e.status.turn_open,
        "one_line_status": e.status.one_line_status,
    })));
    if a.has("state") {
        v["state"] = json!(goals.state(id).await?);
    }
    if let Some(n) = a.num::<usize>("activations")? {
        v["activations"] = json!(goals.activations(id, &ActivationQuery { epoch: None, limit: n }).await?);
    }
    if let Some(n) = a.num::<usize>("changes")? {
        v["changes"] = json!(goals.changes(id, n).await?);
    }
    if a.has("revisions") {
        let mut revs = Vec::new();
        for r in 1..=goal.revision {
            revs.push(goals.revision(id, r).await?);
        }
        v["revisions"] = json!(revs);
    }
    if a.has("epochs") {
        let mut epochs = Vec::new();
        for e in &goal.epochs {
            epochs.push(json!({ "epoch": e, "state": goals.epoch_state(id, e.epoch).await? }));
        }
        v["epochs"] = json!(epochs);
    }
    if a.has("schedule") {
        v["schedule"] = json!(goals.schedule(id).await?);
    }
    let works = c
        .agent
        .sessions()
        .query(&RegistryQuery {
            goal_id: Some(id.to_string()),
            ..Default::default()
        })
        .await?;
    v["works"] = json!(works
        .iter()
        .filter(|e| e.kind != SessionKind::Goal)
        .map(brief)
        .collect::<Vec<_>>());
    Ok(v)
}

async fn cmd_goal(mut a: Args) -> R<i32> {
    let sub = a.next("goal command")?;
    if sub == "serve" {
        return cmd_goal_serve(a).await;
    }
    let c = ctx(&a, None, sub == "skills" || sub == "activate" || sub == "post").await?;
    let caller = goal_caller(&c);
    let goals = c.agent.goals();
    match sub.as_str() {
        "create" => {
            let mut req: GoalCreate = match a.get("json") {
                Some(j) => json_arg(&j)?,
                None => GoalCreate {
                    objective: a.get("objective").ok_or_else(|| bad("missing --objective"))?,
                    ..Default::default()
                },
            };
            if let Some(t) = a.get("objective") {
                req.objective = t;
            }
            if let Some(t) = a.get("title") {
                req.title = Some(t);
            }
            req.key = a.get("key").or(req.key);
            match a.get("lifecycle").as_deref() {
                None => {}
                Some("achieve") => req.lifecycle = GoalLifecycle::Achieve,
                Some("maintain") => req.lifecycle = GoalLifecycle::Maintain,
                Some(s) => return Err(bad(format!("bad --lifecycle {s} (achieve | maintain)"))),
            }
            if let Some(t) = a.get("achieved-when") {
                req.achieved_when = Some(t);
            }
            if let Some(t) = a.get("healthy-state") {
                req.healthy_state = Some(t);
            }
            if let Some(t) = a.get("distance") {
                req.distance_evaluation = t;
            }
            req.constraints.extend(a.all("constraint"));
            req.initial_skills.extend(skills_arg(&a));
            if let Some(w) = a.get("workspace") {
                req.scope.workspace = Some(w);
            }
            req.scope.paths.extend(a.all("path"));
            req.scope.objects.extend(a.all("object"));
            if let Some(n) = a.num::<u64>("every")? {
                req.schedule.every_secs = Some(n);
            }
            if let Some(n) = a.num::<u64>("min-interval")? {
                req.schedule.min_interval_secs = Some(n);
            }
            if a.has("start") {
                req.schedule.start_on_create = true;
            }
            if let Some(b) = a.get("behavior") {
                req.behavior = Some(b);
            }
            if let Some(b) = a.get("budget") {
                req.budget = json_arg(&b)?;
            }
            print(&goals.create(&req, &caller).await?);
        }
        "list" => {
            let list = goals.list().await?;
            print(
                &list
                    .iter()
                    .map(|g| json!({
                        "goal_id": g.goal_id, "title": g.title, "status": g.status,
                        "revision": g.revision, "epoch": g.session.epoch, "session_id": g.session.sid,
                        "every_secs": g.schedule.every_secs, "lifecycle": g.lifecycle,
                    }))
                    .collect::<Vec<_>>(),
            );
        }
        "show" => {
            let id = goal_id_arg(&c, &mut a).await?;
            print(&goal_show(&c, &a, &id).await?);
        }
        "revise" => {
            let id = a.next("<goal_id>")?;
            let mut req: GoalRevise = match a.get("json") {
                Some(j) => json_arg(&j)?,
                None => GoalRevise::default(),
            };
            match a.num::<u64>("expected-revision")? {
                Some(n) => req.expected_revision = n,
                None if a.get("json").is_some() => {}
                None => return Err(bad("missing --expected-revision (read `goal show` first)")),
            }
            if let Some(t) = a.get("objective") {
                req.objective = Some(t);
            }
            if let Some(t) = a.get("title") {
                req.title = Some(t);
            }
            if let Some(t) = a.get("distance") {
                req.distance_evaluation = Some(t);
            }
            if let Some(n) = a.num::<u64>("every")? {
                let mut s = goals
                    .get(&id)
                    .await?
                    .ok_or_else(|| bad(format!("goal {id} does not exist")))?
                    .schedule;
                s.every_secs = Some(n);
                req.schedule = Some(s);
            }
            if let Some(b) = a.get("budget") {
                req.budget = Some(json_arg(&b)?);
            }
            if let Some(r) = a.get("reason") {
                req.reason = Some(r);
            }
            print(&goals.revise(&id, &req, &caller).await?);
        }
        "pause" | "resume" | "archive" | "achieved" => {
            let id = goal_id_arg(&c, &mut a).await?;
            let status = match sub.as_str() {
                "pause" => GoalStatus::Paused,
                "resume" => GoalStatus::Active,
                "archive" => GoalStatus::Archived,
                _ => GoalStatus::Achieved,
            };
            let req = GoalStatusChange {
                status,
                reason: a.get("reason"),
                evidence: a.get("evidence"),
            };
            print(&goals.set_status(&id, &req, &caller).await?);
        }
        "reopen" => {
            let id = a.next("<goal_id>")?;
            let req = GoalReopen {
                reason: a.get("reason").ok_or_else(|| bad("missing --reason"))?,
                fresh_state: a.has("fresh-state"),
            };
            print(&goals.reopen(&id, &req, &caller).await?);
        }
        "restart" => {
            let id = a.next("<goal_id>")?;
            let req = GoalRestart {
                reason: a.get("reason").ok_or_else(|| bad("missing --reason"))?,
                carry_state: a.has("carry-state"),
            };
            print(&goals.restart(&id, &req, &caller).await?);
        }
        "activate" | "post" => {
            let id = a.next("<goal_id>")?;
            let goal = goals
                .get(&id)
                .await?
                .ok_or_else(|| bad(format!("goal {id} does not exist")))?;
            if !goal.status.is_live() {
                return Err(OpenDanError::GoalConflict(format!("goal {id} is {}", goal.status.as_str())).into());
            }
            if c.agent.sessions().lookup(&goal.session.sid).await?.is_none() {
                return Err(bad(format!(
                    "the goal session of {id} does not exist yet: its host creates it (the agent's Loader, or `xagent goal serve`)"
                )));
            }
            let from = env_sid()
                .map(|s| format!("session:{s}"))
                .unwrap_or_else(|| c.who.clone());
            if sub == "activate" {
                let key = crate::goal::activate_key(&id, &uuid::Uuid::new_v4().simple().to_string());
                let note = a.get("reason").unwrap_or_else(|| format!("activated by {from}"));
                let index = crate::goal::post_activation(
                    c.agent.as_ref(),
                    &goal,
                    &c.who,
                    ActivationReasonKind::Manual,
                    &key,
                    &note,
                )
                .await?;
                print(&json!({ "posted": index, "key": key, "session_id": goal.session.sid }));
            } else {
                let text = a.get("msg").ok_or_else(|| bad("missing --msg"))?;
                let (index, key) =
                    crate::goal::post_feedback(c.agent.as_ref(), &goal, &c.who, &text, &from).await?;
                print(&json!({ "posted": index, "key": key, "session_id": goal.session.sid }));
            }
        }
        "skills" => {
            // A change of the Goal State: applied by the session's driver.
            let sid = env_sid().ok_or_else(|| bad("goal skills runs inside a goal session ($OPENDAN_SESSION_ID)"))?;
            let op = a.next("add | remove")?;
            let name = a.next("<name>")?;
            let cmd = match op.as_str() {
                "add" => {
                    let (n, r) = match name.split_once('=') {
                        Some((n, r)) => (n.to_string(), Some(r.to_string())),
                        None => (name.clone(), None),
                    };
                    ControlCommand::GoalSkills {
                        add: vec![GoalSkill {
                            name: n,
                            reference: r,
                            version: a.get("version"),
                            note: a.get("note"),
                        }],
                        remove: Vec::new(),
                    }
                }
                "remove" => ControlCommand::GoalSkills {
                    add: Vec::new(),
                    remove: vec![name.clone()],
                },
                _ => return Err(bad("goal skills add | remove")),
            };
            let key = format!("goal_skills:{}", uuid::Uuid::new_v4().simple());
            let index = c
                .agent
                .sessions()
                .post_input(&sid, &PostedInput::control(&c.who, key, cmd))
                .await?;
            print(&json!({ "posted": index, "note": "applied by the session before its next step" }));
        }
        other => return Err(bad(format!("unknown goal command `{other}`"))),
    }
    Ok(0)
}

/// Host the agent's goals (and every session this identity drives) until
/// interrupted: the Loader's goals module without the Loader.
/// `30`, `30s`, `15m`, `3h`, `2d` → seconds.
fn duration_secs(text: &str) -> R<u64> {
    let t = text.trim();
    let (num, unit) = match t.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => (&t[..i], &t[i..]),
        None => (t, "s"),
    };
    let n: u64 = num.parse().map_err(|_| bad(format!("bad duration {text}")))?;
    let mul = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(bad(format!("bad duration {text} (<n>[s|m|h|d])"))),
    };
    Ok(n.saturating_mul(mul))
}

fn notify_brief(v: &crate::notify::NotifyView) -> Value {
    json!({
        "notify_id": v.notify_id,
        "url": v.url,
        "kind": v.kind,
        "status": v.status,
        "category": v.category,
        "level": v.level,
        "title": v.prompt.title,
        "source": v.source,
        "result": v.result,
        "closed_reason": v.closed_reason,
        "deliveries": v.deliveries,
    })
}

/// The open Turn and the input it is answering, of the session this
/// command runs in.
fn session_position() -> (Option<u64>, Option<String>) {
    let Some(state) = std::env::var("OPENDAN_SESSION_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .and_then(|d| SessionDir::open(&d).ok())
        .and_then(|sd| sd.state().ok())
    else {
        return (None, None);
    };
    let input = state
        .open_turn
        .as_ref()
        .and_then(|t| t.inputs.last().cloned());
    (Some(state.current_turn()), input)
}

async fn cmd_notify(mut a: Args) -> R<i32> {
    use crate::notify::*;
    let sub = a.next("notify command")?;
    let c = ctx(&a, None, false).await?;
    // The host's service as the Agent State carries it (files with --dev,
    // the host over kRPC); inside a session of a zone-hosted agent the
    // zone's msg-center, as the agent's app.
    let notify: Arc<dyn NotifyHost> = match StateNotify::of(c.agent.clone()) {
        Some(n) => n,
        None => match ZoneNotify::for_session_process().await {
            Ok(z) => Arc::new(z),
            Err(why) => {
                return Err(Fail {
                    code: 1,
                    msg: format!("notifications are not available here: {why}"),
                })
            }
        },
    };
    let inside = env_sid();
    let agent_did = c.agent.agent_did().to_string();
    // A request about a notification that is not open any more prints its
    // state and exits 5.
    let conflict = |e: OpenDanError| -> Fail {
        match &e {
            OpenDanError::NotifyConflict(state) => {
                println!("{state}");
                Fail { code: 5, msg: "the notification is closed".into() }
            }
            _ => e.into(),
        }
    };
    match sub.as_str() {
        "send" => {
            let sid = inside.ok_or_else(|| bad("notify send runs inside a session ($OPENDAN_SESSION_ID)"))?;
            let title = a.get("title").ok_or_else(|| bad("missing --title"))?;
            let level: Option<NotifyLevel> = a
                .get("level")
                .map(|l| serde_json::from_value(json!(l)).map_err(|_| bad(format!("bad --level {l}"))))
                .transpose()?;
            let expires_at_ms = a
                .get("expires-in")
                .map(|d| duration_secs(&d))
                .transpose()?
                .map(|s| crate::now_ms() + s * 1000);
            let call_id = std::env::var(agent_tool::llm_bash::ENV_CALL_ID).ok().filter(|c| !c.is_empty());
            let (turn, _) = session_position();
            let key = a.get("key").unwrap_or_else(|| {
                format!(
                    "{sid}:{}:notify:{}",
                    turn.unwrap_or(0),
                    call_id.clone().unwrap_or_else(|| uuid_like())
                )
            });
            let mut req = NotifySendReq::notice(String::new(), title, a.get("message").unwrap_or_default(), key);
            req.level = level;
            req.expires_at_ms = expires_at_ms;
            let mut source = NotifySource {
                session_id: Some(sid),
                turn,
                call_id,
                ..Default::default()
            };
            if c.agent.agent_root().is_some() {
                complete_notify_source(c.agent.as_ref(), &mut source).await;
            }
            req.source = Some(source);
            let v = notify.send(req).await?;
            print(&json!({
                "notify_id": v.notify_id, "url": v.url, "status": v.status,
                "category": v.category, "deliveries": v.deliveries,
            }));
            Ok(0)
        }
        "list" => {
            let mut req = NotifyListReq {
                session_id: a.get("session"),
                goal_id: a.get("goal"),
                ..Default::default()
            };
            if a.has("open") || (inside.is_some() && !a.has("any-source")) {
                req.status = Some(vec![NotifyStatus::Open, NotifyStatus::Read]);
            }
            if inside.is_some() && !a.has("any-source") {
                req.agent_did = Some(agent_did);
            }
            let items = notify.list(req).await?;
            print(&items.iter().map(notify_brief).collect::<Vec<_>>());
            Ok(0)
        }
        "show" => {
            let id = a.next("<notify_id>")?;
            print(&notify.state(&id).await?);
            Ok(0)
        }
        "answer" | "ack" | "dismiss" => {
            let id = a.next("<notify_id>")?;
            let current = notify.state(&id).await?;
            let mut answer = NotifyAnswer {
                dismiss: sub == "dismiss" || a.has("dismiss"),
                option: a.get("option"),
                text: a.get("text"),
            };
            match (sub.as_str(), current.kind) {
                ("answer", NotifyKind::Notice) => {
                    return Err(bad(format!("{id} is a notice: acknowledge it with `notify ack {id}`")))
                }
                ("ack", NotifyKind::Input) => {
                    return Err(bad(format!("{id} asks a question: answer it with `notify answer {id} --option|--text`")))
                }
                ("ack", _) => answer = NotifyAnswer::default(),
                _ => {}
            }
            let via = match &inside {
                Some(sid) => {
                    if sub == "dismiss" || answer.dismiss {
                        return Err(bad("dismissing is the user's decision"));
                    }
                    if current.source.session_id.as_deref() == Some(sid.as_str()) {
                        print(&notify_brief(&current));
                        return Err(Fail { code: 6, msg: "this session sent the notification: it cannot answer it for the user".into() });
                    }
                    if current.status.is_closed() {
                        print(&notify_brief(&current));
                        return Err(Fail { code: 5, msg: "the notification is closed".into() });
                    }
                    let (_, input) = session_position();
                    Some(NotifyVia {
                        agent_did: agent_did.clone(),
                        session_id: Some(sid.clone()),
                        input,
                    })
                }
                None => None,
            };
            if current.kind == NotifyKind::Input && !answer.dismiss {
                if let Err(e) = answer.validate(&current.prompt) {
                    print(&notify_brief(&current));
                    return Err(bad(e));
                }
            }
            let v = notify.answer(&id, answer, via).await.map_err(conflict)?;
            print(&notify_brief(&v));
            Ok(0)
        }
        "cancel" => {
            let id = a.next("<notify_id>")?;
            if inside.is_some() {
                let current = notify.state(&id).await?;
                if current.source.agent_did.as_deref() != Some(agent_did.as_str()) {
                    print(&notify_brief(&current));
                    return Err(Fail { code: 6, msg: "only notifications of this agent can be canceled".into() });
                }
            }
            let reason = a.get("reason").unwrap_or_else(|| "canceled".into());
            print(&notify_brief(&notify.cancel(&id, &reason).await?));
            Ok(0)
        }
        other => Err(bad(format!("unknown notify command `{other}`"))),
    }
}

fn uuid_like() -> String {
    format!("{:x}", crate::now_ms())
}

async fn cmd_goal_serve(a: Args) -> R<i32> {
    let c = ctx(&a, None, true).await?;
    let root = c
        .agent
        .agent_root()
        .ok_or_else(|| bad("goal serve needs the AgentRoot (--agent-root / $OPENDAN_AGENT_ROOT)"))?
        .to_path_buf();
    let llm_context = match a.get("llm-context") {
        None => json!({ "tools": { "enabled": true } }),
        Some(s) => serde_json::from_str(&file_or_inline(&s)?)?,
    };
    let h = host(&c, &a)?;
    let idle = a.num::<u64>("idle-unload")?.map(Duration::from_secs);
    let sup = Supervisor::new(h, idle).with_goals(GoalHost::new(GoalHostOptions::new(
        root.join("sessions"),
        llm_context,
    )));
    let task = tokio::spawn(sup.clone().run());
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("xagent: interrupted, hosting ends (sessions keep their committed state)");
    sup.shutdown().await;
    task.abort();
    let _ = task.await;
    print(&json!({ "kind": "interrupted", "goals": sup.goals().map(|g| g.status()) }));
    Ok(4)
}

// ---------------------------------------------------------------------------
// A Session process on a runtime without the agent's environment (a node
// session-runtime, primary-runtime TODO §6.1): its session is the context
// in its working directory; what needs the control side goes to
// AgentService over kRPC with the session's token.
// ---------------------------------------------------------------------------

/// The Session context of this process, when it has no agent environment:
/// `--session-context <path>` (taken out of `argv`), else the nearest
/// `.opendan/session.json` above the current directory.
fn remote_context(argv: &mut Vec<String>) -> Option<(PathBuf, crate::session_context::SessionContext)> {
    let explicit = argv.iter().position(|a| a == "--session-context").map(|i| {
        let path = argv.get(i + 1).cloned().unwrap_or_default();
        argv.drain(i..(i + 2).min(argv.len()));
        PathBuf::from(path)
    });
    let local = [
        "OPENDAN_SESSION_ID",
        crate::state::ENV_AGENT_ROOT,
        crate::state::ENV_AGENT_STATE_URL,
    ]
    .iter()
    .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()));
    if local && explicit.is_none() {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;
    crate::session_context::discover(explicit.as_deref(), &cwd)
}

/// File arguments are read where the command runs: `--content-file <p>`
/// becomes `--content <text>`, `@<file>` values their text, and a `-` value
/// takes this process's stdin along.
fn inline_files(argv: Vec<String>) -> R<(Vec<String>, Option<String>)> {
    let mut out = Vec::with_capacity(argv.len());
    let mut stdin = None;
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        if arg == "--content-file" {
            let path = it.next().ok_or_else(|| bad("--content-file needs a path"))?;
            out.push("--content".to_string());
            out.push(std::fs::read_to_string(&path).map_err(|e| bad(format!("{path}: {e}")))?);
        } else if arg == "-" && stdin.is_none() {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            stdin = Some(text);
            out.push(arg);
        } else if let Some(file) = arg.strip_prefix('@').filter(|f| Path::new(f).is_file()) {
            out.push(std::fs::read_to_string(file)?);
        } else {
            out.push(arg);
        }
    }
    Ok((out, stdin))
}

async fn remote_main(remote: crate::session_context::SessionClient, cmd: String, argv: Vec<String>) -> R<i32> {
    let sid = remote.ctx.session_id.clone();
    if cmd == "task" {
        use agent_tool::task_bridge::{TaskBridgeRequest, TaskBridgeResponse, TaskOp};
        let mut a = Args::parse(argv, &[]);
        let op = match a.next("state | wait | cancel")?.as_str() {
            "state" => TaskOp::State,
            "wait" => TaskOp::Wait,
            "cancel" => TaskOp::Cancel,
            other => return Err(bad(format!("task takes state | wait | cancel, not `{other}`"))),
        };
        let request = TaskBridgeRequest {
            op,
            task_id: a.next("<task_id>")?,
            wait_ms: a.num("wait-ms")?,
        };
        let v = remote
            .call("session.task", json!({ "sid": sid, "request": request }))
            .await?;
        let resp: TaskBridgeResponse = serde_json::from_value(v)?;
        println!("{}", resp.text);
        return Ok(resp.exit_code());
    }
    if cmd == "skill" && argv.first().map(String::as_str) == Some("install") {
        return Err(bad(
            "skill install reads the package where the command runs, and this runtime is not the agent's: \
install it from a session on the agent's own runtime, or with `buckyos skill install`",
        ));
    }
    let (argv, stdin) = inline_files(argv)?;
    let mut full = vec![cmd];
    full.extend(argv);
    let request = crate::session_context::HelperRequest {
        argv: full,
        stdin,
        call_id: env_call_id(),
    };
    let v = remote
        .call("session.helper", json!({ "sid": sid, "request": request }))
        .await?;
    let out: crate::session_context::HelperOutput = serde_json::from_value(v)?;
    print!("{}", out.stdout);
    eprint!("{}", out.stderr);
    Ok(out.exit_code)
}

async fn real_main() -> R<i32> {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return Ok(if argv.is_empty() { 2 } else { 0 });
    }
    let cmd = argv.remove(0);
    if let Some((path, sc)) = remote_context(&mut argv) {
        // What only reads this machine runs here; the rest is the session's
        // business on the control side.
        match (cmd.as_str(), argv.first().map(String::as_str)) {
            ("ask-user", _) => std::env::set_var("OPENDAN_SESSION_ID", &sc.session_id),
            ("runtime", Some("baseline")) => {}
            _ => {
                let client = crate::session_context::SessionClient { path, ctx: sc };
                return remote_main(client, cmd, argv).await;
            }
        }
    }
    // `--run` is a flag of `status` and takes a run id for `xllm`;
    // `--accept` is a verdict of `todo review` and a criterion elsewhere.
    let extra: &[&str] = match (cmd.as_str(), argv.first().map(String::as_str)) {
        ("status", _) => &["run"],
        ("todo", Some("review")) => &["accept", "reject", "need-evidence", "json"],
        ("todo", _) => &["json"],
        ("skill", _) => &["json", "replace"],
        ("runtime", Some("baseline")) => &["json", "check", "overlay"],
        ("workspace" | "runtime", Some("list")) | ("sessions", _) => &["json"],
        ("copy", _) => &["replace"],
        _ => &[],
    };
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
        "workspace" => cmd_workspace(a).await,
        "runtime" => cmd_runtime(a).await,
        "recall" => cmd_recall(a).await,
        "note" => cmd_note(a).await,
        "sessions" => cmd_sessions(a).await,
        "read-session" => cmd_read_session(a).await,
        "artifact" => cmd_artifact(a).await,
        "create-worksession" => cmd_create_worksession(a).await,
        "ask-user" => cmd_ask_user(a),
        "wait" => cmd_wait(a).await,
        "goal" => cmd_goal(a).await,
        "notify" => cmd_notify(a).await,
        "todo" => cmd_todo(a).await,
        "task" => cmd_task(a).await,
        "skill" => cmd_skill(a).await,
        "copy" => cmd_copy(a).await,
        other => {
            eprintln!("xagent: unknown command `{other}`\n\n{USAGE}");
            Ok(2)
        }
    }
}

/// The command the `agent-session` wrapper runs. In the aios image `xagent`
/// is a symlink to the multi-call `opendan` executable and `current_exe()`
/// resolves symlinks, so prefer the `xagent` beside it.
fn session_cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let beside = exe
        .parent()?
        .join(format!("xagent{}", std::env::consts::EXE_SUFFIX));
    Some(if beside.exists() { beside } else { exe })
}

/// Run the `xagent` command line and exit with its status.
pub fn main() -> ! {
    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    let code = match rt.block_on(real_main()) {
        Ok(code) => code,
        Err(f) => {
            eprintln!("xagent: {}", f.msg);
            f.code
        }
    };
    std::process::exit(code);
}
