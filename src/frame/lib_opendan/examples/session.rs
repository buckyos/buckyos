//! libopendan development CLI.
//!
//! ```text
//! cargo run -p libopendan --example session -- <command> [options]
//!
//! common options
//!   --agent-root <dir>   AgentRoot (Agent State files)        [$OPENDAN_AGENT_ROOT]
//!   --agent <did>        agent DID                            [$OPENDAN_AGENT_DID]
//!   --queue-dir <dir>    development kmsg queue directory     [$LIBOPENDAN_QUEUE_DIR]
//!   --who <principal>    acting identity app:<appid>@<owner>  [$LIBOPENDAN_WHO]
//!
//! commands
//!   create  --parent <dir> --objective <text> [--kind work|self_improve] [--key <idem>]
//!           [--workspace <abs path>] [--artifact <aid>] [--scope <ref>]...
//!           [--llm-context <json | @file>] [--system <text>] [--tool-plan <name>]
//!   run     <session_dir> [--until finished|idle|outcomes:<n>] [--runtime-id <id>]
//!   read    <sid> [--worklog <n>] [--report]
//!   post    <sid> (--text <t> | --stop | --change <key> <text> | --perception <text>) [--key <k>]
//!   decide  <sid> accept|discard [--note <t>]
//!   active
//!   holder  <session_dir>
//!   activity [--sid <sid>] [--summary <t>] [--touch <ref>]... [--clear]   (inside exec)
//!   perceive [--sid <sid>] <text>                                          (inside exec)
//!   schema  <out_dir>          write the protocol JSON Schemas
//! ```
//!
//! LLM providers come from `session_config.prompt.llm_context` (xllm schema:
//! `provider.type: openai` + `base_url` + `api_key_env`, or `buckyos`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_tool::xllm::XllmDeps;
use libopendan::api::{create_session, SessionSpec};
use libopendan::channel::{KmsgChannels, PollWaker};
use libopendan::protocol::*;
use libopendan::runner::{RunnerDeps, SessionRunner, StopWhen};
use libopendan::runtime::NativeRuntime;
use libopendan::state::AgentStateClient;
use libopendan::{FsAgentStateClient, SessionDir};
use serde_json::{json, Value};

type R<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    pos: VecDeque<String>,
    opts: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse() -> Self {
        let mut pos = VecDeque::new();
        let mut opts = Vec::new();
        let mut it = std::env::args().skip(1).peekable();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                let flag = matches!(name, "stop" | "report" | "clear");
                let val = if flag {
                    None
                } else if name == "change" {
                    // --change <key> <text>
                    let k = it.next();
                    let t = it.next();
                    k.zip(t).map(|(k, t)| format!("{k}\u{0}{t}"))
                } else {
                    it.next()
                };
                opts.push((name.to_string(), val));
            } else {
                pos.push_back(a);
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
        self.get(name).or_else(|| std::env::var(env).ok())
    }

    fn need(&self, name: &str, env: &str) -> R<String> {
        self.env_or(name, env)
            .ok_or_else(|| format!("missing --{name} (or ${env})").into())
    }
}

struct Ctx {
    agent: Arc<FsAgentStateClient>,
    channels: Arc<KmsgChannels>,
    who: String,
    queue_dir: PathBuf,
}

fn ctx(a: &Args) -> R<Ctx> {
    let root = a.need("agent-root", "OPENDAN_AGENT_ROOT")?;
    let did = a.need("agent", "OPENDAN_AGENT_DID")?;
    let queue_dir = PathBuf::from(a.need("queue-dir", "LIBOPENDAN_QUEUE_DIR")?);
    let who = a
        .env_or("who", "LIBOPENDAN_WHO")
        .unwrap_or_else(|| "app:libopendan-dev@local".to_string());
    let channels = Arc::new(KmsgChannels::dir(&queue_dir)?);
    let agent = Arc::new(FsAgentStateClient::open(
        &root,
        &did,
        Some(channels.client()),
        Some(Arc::new(PollWaker)),
    )?);
    Ok(Ctx {
        agent,
        channels,
        who,
        queue_dir,
    })
}

fn print(v: &impl serde::Serialize) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn llm_context_arg(a: &Args) -> R<Value> {
    Ok(match a.get("llm-context") {
        None => json!({ "tools": { "enabled": true } }),
        Some(s) if s.starts_with('@') => serde_json::from_str(&std::fs::read_to_string(&s[1..])?)?,
        Some(s) => serde_json::from_str(&s)?,
    })
}

async fn cmd_create(a: &Args) -> R<()> {
    let c = ctx(a)?;
    let parent = PathBuf::from(a.get("parent").ok_or("missing --parent")?);
    let mut spec = SessionSpec::work(a.get("objective").unwrap_or_default());
    if a.get("kind").as_deref() == Some("self_improve") {
        spec.kind = SessionKind::SelfImprove;
        spec.class = "self_improve".into();
    }
    spec.idempotency_key = a.get("key");
    spec.prompt.llm_context = llm_context_arg(a)?;
    spec.prompt.system_prompt = a.get("system");
    spec.runtime.tool_plan = a.get("tool-plan");
    if let Some(ws) = a.get("workspace") {
        spec.workspace = Some(WorkspaceRef::External { path: ws });
    }
    spec.artifact_id = a.get("artifact");
    let scope = a.all("scope");
    if !scope.is_empty() {
        spec.scope = Some(Scope {
            paths: scope,
            objects: vec![],
        });
    }
    let sd = create_session(&parent, spec, c.agent.as_ref(), &c.who, c.channels.as_ref()).await?;
    print(&json!({ "session_id": sd.sid(), "path": sd.path() }));
    Ok(())
}

async fn cmd_run(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let dir = a.pos.pop_front().ok_or("missing <session_dir>")?;
    let sd = SessionDir::open(&dir)?;
    let until = match a.get("until").as_deref() {
        None | Some("finished") => StopWhen::Finished,
        Some("idle") => StopWhen::Idle,
        Some(s) if s.starts_with("outcomes:") => StopWhen::MaxOutcomes {
            n: s[9..].parse()?,
        },
        Some(s) => return Err(format!("bad --until {s}").into()),
    };
    let runtime_id = a
        .get("runtime-id")
        .unwrap_or_else(|| format!("native:{}", libopendan::runtime::native_host_id()));
    let runtime = Arc::new(NativeRuntime::local(&runtime_id, &c.who));
    // Tools started by exec inherit this (the `agent-session` helper uses it).
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &c.queue_dir);
    std::env::set_var("LIBOPENDAN_WHO", &c.who);
    let mut deps = RunnerDeps::new(
        &c.who,
        c.agent.clone(),
        c.channels.clone(),
        runtime,
        XllmDeps::default(),
    );
    if let Ok(exe) = std::env::current_exe() {
        deps = deps.with_session_cli(exe);
    }
    let r = SessionRunner::new(deps).drive(&sd, until).await;
    print(&r);
    Ok(())
}

async fn cmd_read(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let sid = a.pos.pop_front().ok_or("missing <sid>")?;
    let n: usize = a.get("worklog").map(|s| s.parse()).transpose()?.unwrap_or(0);
    let v = libopendan::read_session(c.agent.as_ref(), &sid, false, a.has("report"), n).await?;
    print(&v);
    Ok(())
}

fn sid_arg(a: &mut Args) -> R<String> {
    a.get("sid")
        .or_else(|| a.pos.pop_front())
        .or_else(|| std::env::var("OPENDAN_SESSION_ID").ok())
        .ok_or_else(|| "missing <sid>".into())
}

fn key(a: &Args, prefix: &str) -> String {
    a.get("key")
        .unwrap_or_else(|| format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
}

async fn cmd_post(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let sid = sid_arg(a)?;
    let input = if let Some(t) = a.get("text") {
        Input::msg(key(a, "msg"), t)
    } else if a.has("stop") {
        Input::control(key(a, "stop"), &ControlCommand::Stop { reason: None })
    } else if let Some(kt) = a.get("change") {
        let (k, t) = kt.split_once('\u{0}').ok_or("--change <key> <text>")?;
        Input::change(k.to_string(), json!({ "text": t }))
    } else if let Some(t) = a.get("perception") {
        Input::perception(key(a, "perc"), json!({ "kind": "observation", "summary": t }))
    } else {
        return Err("post needs --text / --stop / --change / --perception".into());
    };
    let idx = libopendan::post_input(c.agent.as_ref(), &sid, &input, &c.who).await?;
    print(&json!({ "posted": idx }));
    Ok(())
}

async fn cmd_decide(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let sid = a.pos.pop_front().ok_or("missing <sid>")?;
    let d = a.pos.pop_front().ok_or("missing accept|discard")?;
    let input = Input::control(
        key(a, "decide"),
        &ControlCommand::Decide {
            decision: d,
            by: c.who.clone(),
            note: a.get("note"),
        },
    );
    let idx = libopendan::post_input(c.agent.as_ref(), &sid, &input, &c.who).await?;
    print(&json!({ "posted": idx }));
    Ok(())
}

async fn cmd_active(a: &Args) -> R<()> {
    let c = ctx(a)?;
    print(&c.agent.activity().active(None, 50).await?);
    Ok(())
}

fn cmd_holder(a: &mut Args) -> R<()> {
    let dir = a.pos.pop_front().ok_or("missing <session_dir>")?;
    let sd = SessionDir::open(&dir)?;
    print(&sd.holder());
    Ok(())
}

async fn cmd_activity(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let sid = sid_arg(a)?;
    let touch = a
        .all("touch")
        .into_iter()
        .map(|t| Touching {
            kind: if t.starts_with("artifact:") {
                "artifact".into()
            } else {
                "path".into()
            },
            target: t,
            mode: "write".into(),
            since_ms: libopendan::now_ms(),
        })
        .collect();
    let cmd = ControlCommand::Activity {
        summary: a.get("summary"),
        touch,
        clear: a.has("clear"),
    };
    let idx =
        libopendan::post_input(c.agent.as_ref(), &sid, &Input::control(key(a, "activity"), &cmd), &c.who)
            .await?;
    print(&json!({ "posted": idx }));
    Ok(())
}

async fn cmd_perceive(a: &mut Args) -> R<()> {
    let c = ctx(a)?;
    let sid = a
        .get("sid")
        .or_else(|| std::env::var("OPENDAN_SESSION_ID").ok())
        .ok_or("missing --sid")?;
    let text = a.pos.pop_front().ok_or("missing <text>")?;
    let input = Input::perception(key(a, "perc"), json!({ "kind": "observation", "summary": text }));
    let idx = libopendan::post_input(c.agent.as_ref(), &sid, &input, &c.who).await?;
    print(&json!({ "posted": idx }));
    Ok(())
}

fn cmd_schema(a: &mut Args) -> R<()> {
    let out = PathBuf::from(a.pos.pop_front().ok_or("missing <out_dir>")?);
    std::fs::create_dir_all(&out)?;
    for (name, schema) in libopendan::protocol::json_schemas() {
        let p = out.join(format!("{name}.schema.json"));
        std::fs::write(&p, serde_json::to_vec_pretty(&schema)?)?;
        println!("{}", p.display());
    }
    Ok(())
}

async fn real_main() -> R<()> {
    let mut a = Args::parse();
    let cmd = a.pos.pop_front().unwrap_or_default();
    match cmd.as_str() {
        "create" => cmd_create(&a).await,
        "run" => cmd_run(&mut a).await,
        "read" => cmd_read(&mut a).await,
        "post" => cmd_post(&mut a).await,
        "decide" => cmd_decide(&mut a).await,
        "active" => cmd_active(&a).await,
        "holder" => cmd_holder(&mut a),
        "activity" => cmd_activity(&mut a).await,
        "perceive" => cmd_perceive(&mut a).await,
        "schema" => cmd_schema(&mut a),
        _ => {
            eprintln!(
                "usage: session <create|run|read|post|decide|active|holder|activity|perceive|schema> [options]"
            );
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = real_main().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    let _ = Path::new(".");
}
