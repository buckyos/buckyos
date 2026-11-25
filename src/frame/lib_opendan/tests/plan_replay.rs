//! Real-model samples of the plan and its work sessions (notepads/
//! jarvis-plan-work-simple-todo.md): what the prompts are to make of the
//! host's mechanics, which unit tests cover with scripted models. Every case
//! runs the Jarvis package of this repository; the role under test is the real
//! model (the local Zone's AICC), the others are scripted where the case needs
//! a certain situation, and hours pass as simulated time. Only what can be
//! decided without reading is asserted; the transcript of every case goes to
//! `target/plan-replay/<case>.md` and is to be read.
//!
//! ```text
//! cargo test -p libopendan --test plan_replay -- --ignored --nocapture --test-threads=1
//! cargo test -p libopendan --test plan_replay -- --ignored --nocapture stuck
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::AiResponse;
use common::plan::*;
use common::*;
use libopendan::host::HostDeps;
use libopendan::notify::{notify_event, LocalNotify, NotifyHost, NotifyKind, NotifyLevel, NotifyStatus};
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, RunnerDeps, StopSignal, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::todos::{TodoMode, TodoStatus};
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::LLMComputeError;
use serde_json::{json, Value};

const OWNER: &str = "did:bns:alice";
const ASK_TASK: &str = "t-0000000000000000000000000000ask1";

type Script = dyn Fn(&LlmInferenceRequest) -> Option<AiResponse> + Send + Sync;

/// The real model, unless the script answers the request.
struct Mixed {
    real: agent_tool::xllm::AiccLlmClient,
    script: Box<Script>,
    real_calls: AtomicUsize,
}

#[async_trait]
impl LlmClient for Mixed {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        match (self.script)(&req) {
            Some(r) => Ok(r),
            None => {
                self.real_calls.fetch_add(1, Ordering::SeqCst);
                self.real.infer(req).await
            }
        }
    }
}

async fn mixed(script: impl Fn(&LlmInferenceRequest) -> Option<AiResponse> + Send + Sync + 'static) -> Arc<Mixed> {
    agent_tool::xllm::ensure_buckyos_runtime().await.expect("the local Zone");
    Arc::new(Mixed {
        real: agent_tool::xllm::AiccLlmClient::new(),
        script: Box::new(script),
        real_calls: AtomicUsize::new(0),
    })
}

fn setup() -> Env {
    let env = match std::env::var("PLAN_REPLAY_KEEP") {
        Ok(dir) => Env::at(&PathBuf::from(dir).join(format!("{}", libopendan::now_ms()))),
        Err(_) => Env::new(),
    };
    install_jarvis(&env);
    copy_dir(&Path::new(PACKAGE).join("plan_skills"), &env.agent_root.join("plan_skills"));
    tool_env(&env);
    // `ask_user` of the cases: what a Zone would print, and no Zone asked.
    let tools = env.agent_root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let envelope = json!({ "agent_tool_protocol": "1", "status": "pending", "task_id": ASK_TASK,
        "pending_reason": "user_input", "summary": "asked the user", "detail": {} });
    std::fs::write(
        tools.join("ask_user"),
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$OPENDAN_AGENT_ROOT/asked.txt\"\ncat <<'EOT'\n{envelope}\nEOT\n"),
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(tools.join("ask_user"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    env
}

fn real_context() -> Value {
    json!({ "model": "llm.chat", "max_tokens": 8192, "provider": { "type": "buckyos" }, "tools": { "enabled": true } })
}

/// The deps the host drives `sd` with.
async fn deps(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>) -> RunnerDeps {
    host_of(env, llm.clone()).runner_deps(sd, StopSignal::default()).await.unwrap()
}

fn host_of(env: &Env, llm: Arc<Mixed>) -> HostDeps {
    let mut h = host(env, llm);
    h.options.max_wait = Duration::from_secs(1200);
    h
}

async fn plan(env: &Env, objective: &str, workspace: Option<&Path>) -> SessionDir {
    let mut spec = plan_spec(env, objective);
    spec.prompt.llm_context = real_context();
    if let Some(dir) = workspace {
        std::fs::create_dir_all(dir).unwrap();
        spec.workspace = Some(env.workspace(dir).await);
    }
    env.create_work(spec).await
}

/// One look at a session, its sub sessions not driven.
async fn look(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>) -> DriveResult {
    look_with(&deps(env, sd, llm).await, sd).await
}

/// The same with the host's deps kept: its task manager knows the commands
/// that run.
async fn look_with(deps: &RunnerDeps, sd: &SessionDir) -> DriveResult {
    Box::pin(drive(sd, deps, StopWhen::Idle)).await
}

/// Drive a session and its sub sessions to its end.
async fn finish(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>) -> DriveResult {
    Box::pin(libopendan::host::run_session(
        &host_of(env, llm.clone()),
        sd,
        StopWhen::Finished,
        StopSignal::default(),
        false,
    ))
    .await
    .result
}

/// The shell commands of a session, in order.
fn commands(sd: &SessionDir) -> Vec<String> {
    read_worklog(sd)
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::AssistantMessage { tool_calls, .. } => Some(tool_calls.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|c| c.args.get("command").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// What a session was given and said, as text.
fn transcript(sd: &SessionDir) -> String {
    let mut out = format!("## session {}\n\n", sd.sid());
    for e in read_worklog(sd) {
        match e.body {
            WorklogBody::UserMessage { content, .. } => out.push_str(&format!("### input\n\n```\n{content}\n```\n\n")),
            WorklogBody::AssistantMessage { assistant, tool_calls, .. } => {
                if !assistant.trim().is_empty() {
                    out.push_str(&format!("### reply\n\n{assistant}\n\n"));
                }
                for c in tool_calls {
                    out.push_str(&format!(
                        "### call `{}`{}\n\n```\n{}\n```\n\n",
                        c.tool,
                        c.intent.map(|i| format!(" — {i}")).unwrap_or_default(),
                        c.args.get("command").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| serde_json::to_string_pretty(&c.args).unwrap())
                    ));
                }
            }
            WorklogBody::ActionResult { result, .. } => {
                let text: String = result.to_string().chars().take(1500).collect();
                out.push_str(&format!("### result\n\n```\n{text}\n```\n\n"));
            }
            WorklogBody::ControlApplied { command, detail, .. } if command == "perceive" => {
                out.push_str(&format!("### perceived\n\n```\n{detail}\n```\n\n"));
            }
            _ => {}
        }
    }
    out
}

fn write(case: &str, text: String) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/plan-replay");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{case}.md")), &text).unwrap();
    println!("{text}");
}

fn todo_table(sd: &SessionDir) -> String {
    let list = list_of(sd.path());
    let mut out = String::from("## TodoList\n\n");
    for t in &list.todos {
        out.push_str(&format!(
            "- {} [{} {}] after={:?} accept={:?} skills={:?} runtime={:?} workspace={:?}\n  {}\n\n{}\n\n",
            t.todo_id,
            t.mode.as_str(),
            t.status.as_str(),
            t.after,
            t.accept,
            t.skills,
            t.runtime_id,
            t.workspace,
            t.title,
            t.content
        ));
    }
    out
}

async fn child_of(env: &Env, sd: &SessionDir, id: &str) -> SessionDir {
    let list = list_of(sd.path());
    let sid = children_of(&list, id).pop().unwrap_or_else(|| panic!("{id} has no work session"));
    SessionDir::open(&env.agent().sessions().lookup(&sid).await.unwrap().unwrap().location).unwrap()
}

fn report_call(report: Value) -> AiResponse {
    tool_call("w-end", "report", report)
}

fn is_work(req: &LlmInferenceRequest) -> bool {
    matches!(role(req), Role::Work | Role::Explore)
}

// ---------------------------------------------------------------------------
// D2 / D3: telling advancing from stuck; waiting for something outside
// ---------------------------------------------------------------------------

/// Two Todos run: one reports a new step at every tick, the other the same
/// sentence for hours. The plan lets the first run and deals with the second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn stuck_or_advancing() {
    let env = setup();
    let sd = plan(
        &env,
        "两件互不依赖的事：1) 把 /mnt/array 的 12 块磁盘镜像逐块拷到备份机（每块大约 20 分钟）；2) 从内网源 http://mirror.lan/idx 下载索引并解压。两件都完成后报告。",
        None,
    )
    .await;
    let disk = Arc::new(AtomicUsize::new(1));
    let step = disk.clone();
    let llm = mixed(move |req| {
        Some(match role(req) {
            // The plan's first Turn is scripted: the situation is the case.
            Role::Plan if has_tool_result(req, "p1").is_none() => shell(
                "p1",
                "Two independent pieces of work.",
                "agent-session todo add \"拷贝磁盘镜像\" --after none --content \"把 /mnt/array 的 12 块磁盘镜像逐块拷到备份机，每块约 20 分钟。\" --accept \"12 块的校验和一致\" && agent-session todo add \"下载索引\" --after none --content \"从 http://mirror.lan/idx 下载索引并解压。\" --accept \"索引文件存在且能解压\"",
            ),
            Role::Plan if right_after(req, "p1") => text("两件都在跑。"),
            Role::Plan => return None,
            _ => {
                let objective = system_of(req);
                if has_tool_result(req, "w1").is_none() {
                    return Some(tool_call("w1", "shell", json!({ "command": "for i in $(seq 1 6000); do test -f done && exit 0; sleep 0.1; done" })));
                }
                if objective.contains("拷贝磁盘镜像") {
                    let n = step.fetch_add(1, Ordering::SeqCst);
                    text(&format!("第 {n} 块拷完，校验和一致；第 {} 块在拷。", n + 1))
                } else {
                    text("下载停在 3%，连接 mirror.lan 没有响应，没有新的输出。")
                }
            }
        })
    })
    .await;
    look(&env, &sd, &llm).await;
    let (a, b) = (child_of(&env, &sd, "T01").await, child_of(&env, &sd, "T02").await);
    for c in [&a, &b] {
        shorten_wait(c);
    }
    let works = [deps(&env, &a, &llm).await, deps(&env, &b, &llm).await];
    for (c, d) in [&a, &b].into_iter().zip(&works) {
        look_with(d, c).await;
    }
    let mut decided = None;
    for round in 1..=4u32 {
        for _ in 0..(6 << (round - 1)).min(24) {
            for (c, d) in [&a, &b].into_iter().zip(&works) {
                pass_time(c, 600);
                if !c.state().unwrap().is_finished() {
                    look_with(d, c).await;
                }
            }
            pass_time(&sd, 600);
            if round > 1 || sd.state().unwrap().task_watch.ticks == 0 {
                look(&env, &sd, &llm).await;
            }
        }
        let cmds = commands(&sd).join("\n");
        if decided.is_none() && (cmds.contains("todo stop T02") || cmds.contains("todo update T02") || cmds.contains("todo cancel T02")) {
            decided = Some(round);
            break;
        }
    }
    let cmds = commands(&sd).join("\n");
    write("stuck_or_advancing", format!("decided in round {decided:?}\n\n{}{}", todo_table(&sd), transcript(&sd)));
    for c in [&a, &b] {
        std::fs::write(c.path().join("done"), "").ok();
    }
    assert!(!cmds.contains("todo stop T01") && !cmds.contains("todo cancel T01"), "T01 advanced: it is let run");
    assert!(decided.is_some(), "T02 never moved: the plan has to deal with it");
}

/// Commands of this session become tasks after 300 ms.
fn shorten_wait(sd: &SessionDir) {
    let path = sd.file(SESSION_CONFIG_FILE);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["prompt"]["llm_context"]["runtime"] = json!({ "task_wait_ms": 300 });
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

fn right_after(req: &LlmInferenceRequest, id: &str) -> bool {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == buckyos_api::AiRole::Assistant)
        .is_some_and(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, buckyos_api::AiContent::ToolUse { call_id, .. } if call_id == id))
        })
}

/// A file somebody else delivers tomorrow. The plan waits for it with few
/// inferences and no Todo, and works once it is there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn waits_for_a_file() {
    let env = setup();
    let project = env.root.join("project");
    let sd = plan(
        &env,
        "供应商大约明天上午会把 incoming/report.csv 放进 workspace（由他们的系统推送，不需要我们做什么）。文件到了之后，统计它的数据行数（不含表头），把结果写进 workspace 的 summary.md（一行：`rows: <n>`），然后报告行数。",
        Some(&project),
    )
    .await;
    let llm = mixed(|_| None).await;
    look(&env, &sd, &llm).await;
    let first = llm.real_calls.load(Ordering::SeqCst);
    let st = sd.state().unwrap();
    let mut at = Vec::new();
    if !st.is_finished() {
        for step in 1..=24 * 6 {
            pass_time(&sd, 600);
            let before = llm.real_calls.load(Ordering::SeqCst);
            look(&env, &sd, &llm).await;
            if llm.real_calls.load(Ordering::SeqCst) > before {
                at.push((step * 10, llm.real_calls.load(Ordering::SeqCst) - before));
            }
            if sd.state().unwrap().is_finished() {
                break;
            }
        }
    }
    let waiting = (sd.state().unwrap().is_finished(), list_of(sd.path()).todos.len());
    std::fs::create_dir_all(project.join("incoming")).unwrap();
    std::fs::write(project.join("incoming/report.csv"), "id,name\n1,a\n2,b\n3,c\n").unwrap();
    pass_time(&sd, 4 * 3600);
    let r = finish(&env, &sd, &llm).await;
    let summary = std::fs::read_to_string(project.join("summary.md")).unwrap_or_default();
    write(
        "waits_for_a_file",
        format!("first Turn: {first} inferences; ticks (minute, inferences): {at:?}; finished/todos while waiting: {waiting:?}\nend: {r:?}\nsummary.md: {summary}\n\n{}{}", todo_table(&sd), transcript(&sd)),
    );
    assert!(!waiting.0, "it waits, it does not end");
    assert_eq!(waiting.1, 0, "nothing is dispatched before the file is there");
    assert!(at.len() <= 8, "{at:?}");
    assert!(r.is_finished() && sd.state().unwrap().outcome == Some(Outcome::Succeeded), "{r:?}");
    assert!(summary.contains("rows: 3"), "{summary}");
}

// ---------------------------------------------------------------------------
// D6 / D9: where the verification stands
// ---------------------------------------------------------------------------

/// A change that is released: the release runs after the verification Todo.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn release_after_verification() {
    let env = setup();
    let project = env.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("price.py"), "def total(items):\n    return sum(i['price'] for i in items)\n").unwrap();
    std::fs::write(project.join("test_price.py"), "from price import total\n\ndef test_total():\n    assert total([{'price': 2, 'qty': 3}]) == 6\n").unwrap();
    std::fs::write(project.join("publish.sh"), "#!/bin/sh\n# uploads the package to the public registry; a published version cannot be withdrawn\necho published >> RELEASES\n").unwrap();
    let sd = plan(
        &env,
        "workspace 里的 price.py 漏算数量（qty），test_price.py 现在不过。修好它，然后用 publish.sh 发布（发布后不能撤回，所以发布前要有不是改代码的人做的独立验收）。完成后报告改了哪些文件、验收证据和发布结果。",
        Some(&project),
    )
    .await;
    let llm = mixed(|_| None).await;
    look(&env, &sd, &llm).await;
    write("release_after_verification", format!("{}{}", todo_table(&sd), transcript(&sd)));
    let list = list_of(sd.path());
    let publish: Vec<_> = list.todos.iter().filter(|t| t.mode == TodoMode::Serial && t.title.contains("发布")).collect();
    assert_eq!(publish.len(), 1, "one Todo publishes");
    let before: Vec<_> = list.todos.iter().filter(|t| publish[0].after.contains(&t.todo_id)).collect();
    assert!(
        before.iter().any(|t| t.mode == TodoMode::Explore || t.title.contains("验")),
        "the release runs after a verification: {before:?}"
    );
    assert!(publish[0].attempts.is_empty());
}

/// Something each work session shows by itself: no verification Todo.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn no_verification_for_queries() {
    let env = setup();
    let project = env.root.join("project");
    let sd = plan(
        &env,
        "三件独立的查询，各写一个文件到 workspace：1) 本机有哪些网络接口和地址 → interfaces.md；2) 本机监听的 TCP 端口 → ports.md；3) 本机各磁盘分区的用量 → disks.md。每个文件一张表，完成后报告三个文件的路径。",
        Some(&project),
    )
    .await;
    let llm = mixed(|_| None).await;
    look(&env, &sd, &llm).await;
    write("no_verification_for_queries", format!("{}{}", todo_table(&sd), transcript(&sd)));
    let list = list_of(sd.path());
    assert!(list.todos.len() >= 2, "{}", list.todos.len());
    assert!(verifications(&list).is_empty(), "no Todo only repeats the work's own checks: {:?}", verifications(&list));
}

/// Titles of the Todos that check another's result.
fn verifications(list: &libopendan::todos::TodoList) -> Vec<String> {
    list.todos
        .iter()
        .filter(|t| ["验", "检查", "复核", "核对"].iter().any(|w| t.title.contains(w)) || t.title.to_lowercase().contains("verif"))
        .map(|t| t.title.clone())
        .collect()
}

/// A later Todo verifies: the work session checks what its criteria ask for
/// and no more; without one it checks its result itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn worker_leaves_verification_to_the_verification_todo() {
    let mut out = String::new();
    let mut counts = Vec::new();
    for verified in [true, false] {
        let env = setup();
        let project = env.root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let sd = plan(&env, "Write slug.py and check it.", Some(&project)).await;
        let add = "agent-session todo add \"write slug.py\" --content \"Write slug.py in the workspace with slug(text): lower case, every run of characters other than a-z 0-9 becomes one '-', no '-' at the ends. The report has to contain: the file's path, and in result the fields path and lines (the number of lines).\" --accept \"python3 -c 'import slug; print(slug.slug(\\\"Hello, World!\\\"))' prints hello-world\"";
        let verify = " && agent-session todo explore \"verify slug.py\" --after T01 --content \"Verify slug.py in depth: empty text, only punctuation, unicode, long text, digits, repeated separators, idempotence; run a property test with 10000 random inputs.\"";
        let command = format!("{add}{}", if verified { verify } else { "" });
        let llm = mixed(move |req| {
            Some(match role(req) {
                Role::Plan if has_tool_result(req, "p1").is_none() => shell("p1", "The list.", &command),
                Role::Plan => text("Work runs."),
                _ => return None,
            })
        })
        .await;
        look(&env, &sd, &llm).await;
        let child = child_of(&env, &sd, "T01").await;
        let r = finish(&env, &child, &llm).await;
        let st = child.state().unwrap();
        let cmds = commands(&child);
        out.push_str(&format!("# verified later: {verified}\n\n{r:?}, {} commands\n\n{}", cmds.len(), transcript(&child)));
        let report = st.final_report.clone();
        out.push_str(&format!("### final report\n\n{report:?}\n\n"));
        assert_eq!(st.outcome, Some(Outcome::Succeeded), "{out}");
        let mut result = report.unwrap().result.unwrap_or_default();
        if let Some(text) = result.as_str() {
            result = serde_json::from_str(text).unwrap_or_default();
        }
        assert!(result.get("path").is_some() && result.get("lines").is_some(), "the fields the task named: {result}");
        counts.push((cmds.len(), cmds.join("\n").len()));
    }
    write("worker_leaves_verification", format!("commands (count, bytes) with / without a verification Todo: {counts:?}\n\n{out}"));
}

// ---------------------------------------------------------------------------
// D1: update in place, or stop and retry
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn update_or_stop_and_retry() {
    let env = setup();
    let project = env.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("sales.csv"), "region,amount\nnorth,10.5\nsouth,20.25\nnorth,4.5\n").unwrap();
    let sd = plan(&env, "写一个 Python 命令行脚本 report.py，读 sales.csv，打印每个地区的合计。放在 workspace。", Some(&project)).await;
    let llm = mixed(move |req| {
        Some(match role(req) {
            Role::Plan if has_tool_result(req, "p1").is_none() => shell(
                "p1",
                "One piece of work.",
                "agent-session todo add \"写 report.py\" --content \"写 report.py：读 sales.csv，打印每个地区的合计。放在 workspace。\" --accept \"用样例 sales.csv 运行，输出每个地区一行合计\"",
            ),
            Role::Plan if right_after(req, "p1") => text("在写 report.py。"),
            Role::Plan => return None,
            _ => {
                let n = req.messages.iter().filter(|m| m.role == buckyos_api::AiRole::Tool).count();
                tool_call(&format!("s{n}"), "shell", json!({ "command": "sleep 1" }))
            }
        })
    })
    .await;
    look(&env, &sd, &llm).await;
    let agent = env.agent();
    agent.sessions().post_input(sd.sid(), &msg("另外每个地区的合计请保留两位小数。")).await.unwrap();
    look(&env, &sd, &llm).await;
    let after_detail = commands(&sd).join("\n");
    agent.sessions().post_input(sd.sid(), &msg("改主意了：不要脚本了。直接交付一份 PDF 报告，按地区列出合计和占比，给老板看的。")).await.unwrap();
    look(&env, &sd, &llm).await;
    // The old attempt ends as stopped; the plan hears of it.
    let first = child_of(&env, &sd, "T01").await;
    look(&env, &first, &llm).await;
    look(&env, &sd, &llm).await;
    let all = commands(&sd).join("\n");
    write("update_or_stop_and_retry", format!("{}{}", todo_table(&sd), transcript(&sd)));
    assert!(after_detail.contains("todo update T01") && !after_detail.contains("todo stop"), "a detail is updated in place: {after_detail}");
    assert!(all.contains("todo stop T01") || all.contains("todo cancel T01"), "another deliverable stops the old attempt: {all}");
    let list = list_of(sd.path());
    let t01 = list.get("T01").unwrap();
    assert_eq!(t01.attempts[0].status, TodoStatus::Stopped, "the old attempt never counts as done");
    assert!(t01.attempts.len() > 1 || list.todos.len() > 1, "and the new deliverable is dispatched");
}

// ---------------------------------------------------------------------------
// D8: what the plan remembers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn remembers_an_unexpected_failure() {
    let env = setup();
    let project = env.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "# svc\n\n`make build` builds the service into `out/svc`.\n").unwrap();
    std::fs::write(project.join("Makefile"), "VENDOR_CACHE ?= vendor/cache\n\nbuild:\n\tmkdir -p $(VENDOR_CACHE) out\n\tcc -o out/svc main.c\n").unwrap();
    std::fs::write(project.join("main.c"), "#include <stdio.h>\nint main(void) { puts(\"1.4.2\"); return 0; }\n").unwrap();
    let sd = plan(&env, "在 workspace 里构建服务（README 说用 `make build`），报告构建产物的路径。", Some(&project)).await;
    // vendor/ is managed by somebody else and cannot be written. The first
    // attempt finds it out and may not go on; the later ones are real.
    let vendor = project.join("vendor");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::write(vendor.join("README"), "mirror-sync manages this directory; its permissions are reset every night.\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&vendor, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();
    let dir = project.clone();
    let llm = mixed(move |req| {
        if !is_work(req) || !system_of(req).contains("Todo T01 (attempt 1,") {
            return None;
        }
        Some(match has_tool_result(req, "w1") {
            None => tool_call("w1", "shell", json!({ "command": format!("cd {} && make build; ls -ld vendor; cat vendor/README", dir.display()) })),
            Some(out) => report_call(json!({
                "report": format!("构建失败：\n{out}\n查明原因：Makefile 默认要写 vendor/cache，而这个 workspace 的 vendor/ 是 mirror-sync 管理的只读目录（r-xr-xr-x，权限每晚重置），任何人都不能往里写。Makefile 有 VENDOR_CACHE 变量可以换到别处，但任务只说用 `make build`，我没有自行改参数。README 没写这一点。"),
                "outcome": "failed", "is_end": true
            })),
        })
    })
    .await;
    let r = finish(&env, &sd, &llm).await;
    let records = libopendan::state::perception::read_records(&env.agent().layout().perception_file(sd.sid())).unwrap();
    let report = sd.state().unwrap().final_report.map(|r| r.report).unwrap_or_default();
    let mut out = format!("end: {r:?}\n\n## perceived\n\n");
    for rec in &records {
        out.push_str(&format!("- {}\n  {}\n", rec.summary, serde_json::to_string(rec).unwrap()));
    }
    out.push_str(&format!("\n## final report\n\n{report}\n\n{}{}", todo_table(&sd), transcript(&sd)));
    write("remembers_an_unexpected_failure", out);
    assert!(r.is_finished());
    assert!(!records.is_empty(), "the cause is remembered");
    assert!(records.len() <= 3, "progress is not: {}", records.len());
    assert!(records.iter().any(|r| r.summary.contains("vendor") || r.summary.contains("VENDOR")), "{records:?}");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
    assert!(records.iter().all(|r| serde_json::to_string(r).unwrap().contains("workspace:ws-")), "{records:?}");
    assert!(report.contains("Remembered") || report.contains("记"), "{report}");
}

/// The conversation relays a plan's report: what the plan recorded is not
/// recorded again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn the_conversation_does_not_remember_it_again() {
    let env = setup();
    let (sd, llm) = conversation(&env).await;
    let child = plan_child(&env, &sd, "构建服务").await;
    let done = mixed(|req| {
        assert_eq!(role(req), Role::Plan);
        Some(tool_call("end", "report", json!({
            "intent": "End.",
            "report": "构建成功，产物 out/svc（1.4.2）。\n\n过程：第一次失败，`vendor/` 是只读挂载，默认要写 vendor/cache；改用 `make build VENDOR_CACHE=$TMPDIR/vc` 后通过。\n\nRemembered (already in memory):\n- 这个 workspace 的 vendor/ 是只读挂载的 squashfs，构建要用 `make build VENDOR_CACHE=$TMPDIR/vc`。",
            "outcome": "succeeded", "is_end": true
        })))
    })
    .await;
    look(&env, &child, &done).await;
    look(&env, &sd, &llm).await;
    say(&env, &sd, &llm, "构建怎么样了？").await;
    let records = libopendan::state::perception::read_records(&env.agent().layout().perception_file(sd.sid())).unwrap();
    write(
        "the_conversation_does_not_remember_it_again",
        format!("perceived by the conversation: {:?}\n\n{}", records.iter().map(|r| r.summary.clone()).collect::<Vec<_>>(), transcript(&sd)),
    );
    assert!(records.iter().all(|r| !r.summary.contains("vendor")), "{records:?}");
}

// ---------------------------------------------------------------------------
// D7: asking, and where an answer goes
// ---------------------------------------------------------------------------

/// A one-to-one conversation with its owner; intent analysis is taken out.
async fn conversation(env: &Env) -> (SessionDir, Arc<Mixed>) {
    let route = env.agent_root.join("behaviors/chat_route.toml");
    let t = std::fs::read_to_string(&route).unwrap();
    std::fs::write(&route, t.replace(r#", "behavior:analyze_intent"]"#, "]").replace("next = [\"analyze_intent\"]\n", "")).unwrap();
    let template = SessionTemplate::load("ui", Some(&env.agent_root)).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Aalice"));
    spec.owner = Some(OWNER.into());
    spec.outbound = Some(OutboundBinding { to: OWNER.into(), to_session: None, kind: "chat".into() });
    spec.prompt.llm_context = real_context();
    spec.freeze = true;
    (env.create_work(spec).await, mixed(|_| None).await)
}

/// A plan the conversation started.
async fn plan_child(env: &Env, ui: &SessionDir, title: &str) -> SessionDir {
    libopendan::api::create_sub_session(
        env.agent().as_ref(),
        APP,
        ui.sid(),
        libopendan::api::SubSessionSpec {
            objective: title.into(),
            title: Some(title.into()),
            msgs: vec![title.into()],
            class: Some("plan_simple".into()),
            key: Some(title.into()),
            workspace: libopendan::api::SubWorkspace::None,
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap()
}

async fn say(env: &Env, sd: &SessionDir, llm: &Arc<Mixed>, words: &str) {
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg(words)).await.unwrap();
    let mut d = deps(env, sd, llm).await;
    d.notify = Some(Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap()));
    for _ in 0..2 {
        Box::pin(drive(sd, &d, StopWhen::Idle)).await;
    }
}

/// A running plan asked something. What the user types answers the question
/// once (`notify answer`, not posted as well); a change of the work is posted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn an_answer_goes_to_its_question_a_change_to_the_plan() {
    let env = setup();
    let (sd, llm) = conversation(&env).await;
    let child = plan_child(&env, &sd, "做一张活动海报").await;
    let local = LocalNotify::enable(&env.agent_root, AGENT).unwrap();
    let mut req = buckyos_api::NotifySendReq::input(
        "",
        "海报主色用什么？",
        "",
        vec![buckyos_api::HumanPromptOption::new("blue", "蓝色"), buckyos_api::HumanPromptOption::new("red", "红色")],
        true,
        "plan:ask:1",
    );
    req.source = Some(buckyos_api::NotifySource { session_id: Some(child.sid().to_string()), ..Default::default() });
    let q = local.send(req).await.unwrap();
    let data = buckyos_api::NotifyDeliveredData {
        notify_id: q.notify_id.clone(),
        url: "/messagehub".into(),
        kind: NotifyKind::Input,
        category: buckyos_api::NotifyCategory::Agent,
        level: NotifyLevel::Timely,
        title: "海报主色用什么？".into(),
        message: None,
        options: vec![buckyos_api::HumanPromptOption::new("blue", "蓝色"), buckyos_api::HumanPromptOption::new("red", "红色")],
        source: buckyos_api::NotifySource { session_id: Some(child.sid().to_string()), ..Default::default() },
        delivered_msg_id: None,
        reply_route: buckyos_api::NotifyConversation { to: OWNER.into(), to_session: None },
        at_ms: libopendan::now_ms(),
    };
    let (key, ev) = notify_event(&data, "dm:did:bns:alice");
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &PostedInput::event(APP, key, ev)).await.unwrap();
    say(&env, &sd, &llm, "蓝的吧").await;
    let first = commands(&sd).join("\n");
    let answered = local.state(&q.notify_id).await.unwrap();
    say(&env, &sd, &llm, "对了海报上再加一行地址：中山路 18 号").await;
    let all = commands(&sd).join("\n");
    write("an_answer_goes_to_its_question", format!("question: {:?} {:?}\n\n{}", answered.status, answered.result, transcript(&sd)));
    assert_eq!(answered.status, NotifyStatus::Answered, "{first}");
    assert_eq!(answered.result.unwrap().option(), Some("blue"));
    assert!(first.contains("notify answer") && !first.contains("agent-session post"), "the answer is delivered once: {first}");
    let second = all[first.len()..].to_string();
    assert!(second.contains(&format!("agent-session post {}", child.sid())) && !second.contains("notify answer"), "a change is posted: {second}");
}

/// The plan lacks one thing only the user knows: it asks, runs what does not
/// depend on it, and holds back what does. With everything given it asks
/// nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn asks_for_what_only_the_user_knows() {
    let mut out = String::new();
    for clear in [false, true] {
        let env = setup();
        let project = env.root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("sales.csv"), "region,amount\nnorth,10\nsouth,20\n").unwrap();
        let threshold = if clear { "阈值是 15。" } else { "阈值由我定，开始后问我。" };
        let sd = plan(
            &env,
            &format!("处理 workspace 的 sales.csv，两件独立的事：1) 统计每列的非空行数写进 stats.md；2) 只保留 amount 不低于阈值的行，写进 filtered.csv。{threshold}"),
            Some(&project),
        )
        .await;
        let backend = agent_tool::MemoryTaskBackend::new();
        backend.seed(agent_tool::ZoneTaskRecord {
            task_id: ASK_TASK.into(),
            schema_id: Some("human.input/v1".into()),
            name: "threshold".into(),
            message: Some("threshold".into()),
            ..Default::default()
        });
        let llm = mixed(|req| is_work(req).then(|| report_call(json!({ "report": "完成：文件已写好并核对。", "outcome": "succeeded", "is_end": true })))).await;
        let mut d = deps(&env, &sd, &llm).await;
        d.xllm.buckyos_tasks = Some(agent_tool::ZoneTaskResolver::new(backend.clone()));
        Box::pin(drive(&sd, &d, StopWhen::Idle)).await;
        let asked = std::fs::read_to_string(env.agent_root.join("asked.txt")).unwrap_or_default();
        let list = list_of(sd.path());
        let titles: Vec<String> = list.todos.iter().map(|t| format!("{} {} {}", t.todo_id, t.title, t.content)).collect();
        let before = titles.join("\n");
        let mut after = String::new();
        if !clear && !asked.is_empty() {
            backend
                .complete(ASK_TASK, json!({"action": "submit", "values": {"text": "15"}}))
                .unwrap();
            Box::pin(drive(&sd, &d, StopWhen::Idle)).await;
            after = list_of(sd.path()).todos.iter().map(|t| format!("{} {} {}", t.todo_id, t.title, t.content)).collect::<Vec<_>>().join("\n");
        }
        out.push_str(&format!("# clear: {clear}\n\nasked:\n{asked}\n\ntodos before the answer:\n{before}\n\nafter:\n{after}\n\n{}", transcript(&sd)));
        write("asks_for_what_only_the_user_knows", out.clone());
        if clear {
            assert!(asked.is_empty(), "nothing to ask: {asked}");
            assert!(list.todos.len() >= 2 || before.contains("filtered"), "{before}");
        } else {
            assert!(!asked.is_empty(), "it asks for the threshold");
            assert!(before.contains("stats"), "what does not depend on the answer runs: {before}");
            assert!(list.todos.iter().all(|t| !t.title.contains("过滤") && !t.title.contains("阈值")), "what depends on it waits: {before}");
            assert!(after.contains("filtered") && after.contains("15"), "the answer comes in: {after}");
            assert!(sd.state().unwrap().task_watch.ticks == 0);
        }
    }
}

// ---------------------------------------------------------------------------
// A check that fails for good
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_check_that_fails_for_good_is_reported() {
    let env = setup();
    let project = env.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("input.bin"), "partial download").unwrap();
    std::fs::write(project.join("input.sha256"), "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08  input.bin\n").unwrap();
    std::fs::write(project.join("verify.sh"), "#!/bin/sh\n# do not edit: the release check\necho run >> verify.log\nsha256sum -c input.sha256\n").unwrap();
    let sd = plan(&env, "Verify the input and unpack it.", Some(&project)).await;
    let llm = mixed(|req| {
        Some(match role(req) {
            Role::Plan if has_tool_result(req, "p1").is_none() => shell(
                "p1",
                "One Todo.",
                "agent-session todo add \"校验并登记输入\" --content \"workspace 的 input.bin 是上游给的文件，不能改动，verify.sh 和 input.sha256 也不能改。用 sh verify.sh 校验它，通过后在 workspace 写 accepted.txt（内容 ok）。再把 input.bin 的大小（字节）写进 size.txt。\" --accept \"sh verify.sh 退出码 0\" --accept \"size.txt 里是 input.bin 的字节数\"",
            ),
            Role::Plan => text("Work runs."),
            _ => return None,
        })
    })
    .await;
    look(&env, &sd, &llm).await;
    let child = child_of(&env, &sd, "T01").await;
    let r = finish(&env, &child, &llm).await;
    let runs = std::fs::read_to_string(project.join("verify.log")).unwrap_or_default().lines().count();
    let st = child.state().unwrap();
    write("a_check_that_fails_for_good", format!("{r:?}; verify.sh ran {runs} times\n\n{:?}\n\n{}", st.final_report, transcript(&child)));
    assert_eq!(st.outcome, Some(Outcome::Failed));
    let report = st.final_report.unwrap();
    assert!(matches!(report.outcome, Some(ReportOutcome::Failed | ReportOutcome::Blocked)), "{report:?}");
    assert!((1..=2).contains(&runs), "it ran the same check {runs} times");
    assert!(report.report.contains("sha256") || report.report.contains("校验"), "why: {report:?}");
}

// ---------------------------------------------------------------------------
// What each creator hands over: the same request, from a plan, a
// conversation and a Goal
// ---------------------------------------------------------------------------

const SCAN: &str = "扫描局域网 198.51.100.0/24 上开着 9100 端口的设备，列出来。结果要每台一行（IP、响应时间），另外给一个 JSON 方便导入。不要对设备做任何别的操作，不要改任何设置。";

/// What a created work session was given.
fn given(sd: &SessionDir) -> String {
    let cfg = sd.config().unwrap();
    let mut out = format!(
        "behavior={:?} class={} runtime={} workspace={:?} manuals={:?}\n\nobjective:\n{}\n",
        cfg.prompt.behavior,
        cfg.session.class,
        cfg.runtime.runtime_id(),
        cfg.workspace.as_ref().map(|w| w.workspace_id.clone()),
        cfg.prompt.cmd_skills,
        cfg.session.objective
    );
    for input in &cfg.prompt.initial_inputs {
        if let SessionInput::Msg(m) = &input.input {
            out.push_str(&format!("\nmessage:\n{}\n", m.msg.content.content));
        }
    }
    out
}

fn assert_complete(text: &str) {
    assert!(text.contains("198.51.100.0/24") && text.contains("9100"), "inputs: {text}");
    assert!(text.contains("JSON") && text.contains("IP"), "what the report has to contain: {text}");
    assert!(["设置", "配置", "setting"].iter().any(|w| text.contains(w)), "what it may not change: {text}");
}

async fn works_of(env: &Env, parent: Option<&str>) -> Vec<SessionDir> {
    let mut out = Vec::new();
    for e in env.agent().sessions().query(&RegistryQuery::default()).await.unwrap() {
        let mine = parent.is_none_or(|p| e.origin.as_ref().and_then(|o| o.parent_session.as_deref()) == Some(p));
        if e.kind == SessionKind::Work && mine {
            out.push(SessionDir::open(&e.location).unwrap());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn the_conversation_gives_everything_a_work_session_needs() {
    let env = setup();
    let template = SessionTemplate::load("ui", Some(&env.agent_root)).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Aalice"));
    spec.owner = Some(OWNER.into());
    spec.outbound = Some(OutboundBinding { to: OWNER.into(), to_session: None, kind: "chat".into() });
    spec.prompt.llm_context = real_context();
    spec.freeze = true;
    let sd = env.create_work(spec).await;
    let llm = mixed(|_| None).await;
    say(&env, &sd, &llm, &format!("开一个后台的 work 去做这件事：{SCAN}")).await;
    let works = works_of(&env, Some(sd.sid())).await;
    let out: String = works.iter().map(given).collect();
    write("the_conversation_gives_everything", format!("{out}\n\n{}", transcript(&sd)));
    assert_eq!(works.len(), 1, "one work session");
    assert_eq!(works[0].config().unwrap().prompt.behavior.as_deref(), Some("work_simple"));
    assert_complete(&out);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_goal_gives_everything_a_work_session_needs() {
    use libopendan::host::goals::{GoalHost, GoalHostOptions};
    use libopendan::state::{GoalCaller, GoalCreate};
    let env = setup();
    let llm = mixed(|req| {
        let s = system_of(req);
        (!s.contains("You are the context of one long-lived goal")).then(|| panic!("only the goal runs: {s}"))
    })
    .await;
    let mut opts = GoalHostOptions::new(env.agent_root.join("sessions"), real_context());
    opts.budget_every = Duration::ZERO;
    let mut h = host_of(&env, llm.clone());
    // Only the Goal runs here: the work it creates is looked at, not driven.
    h.max_child_concurrency = 0;
    let sup = libopendan::host::Supervisor::new(h, None).with_goals(GoalHost::new(opts));
    let agent = env.agent();
    let goal = agent
        .goals()
        .create(
            &GoalCreate {
                objective: format!("记录一次扫描结果。第一次激活就建一个 work 做这件事，之后由你汇总：{SCAN}"),
                achieved_when: Some("扫描结果拿到并确认".into()),
                distance_evaluation: "结果是否已经拿到".into(),
                schedule: GoalScheduleSpec { start_on_create: true, min_interval_secs: Some(0), ..Default::default() },
                ..Default::default()
            },
            &GoalCaller::owner(OWNER),
        )
        .await
        .unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(600);
    loop {
        sup.tick().await.unwrap();
        sup.tick_goals().await.unwrap();
        let done = agent.goals().activations(&goal.goal_id, &Default::default()).await.unwrap().iter().any(|a| a.ended_at_ms > 0);
        if done || !works_of(&env, None).await.is_empty() || std::time::Instant::now() > end {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let works = works_of(&env, None).await;
    let out: String = works.iter().map(given).collect();
    sup.shutdown().await;
    let session = agent.sessions().lookup(&goal.session.sid).await.unwrap().unwrap();
    write("a_goal_gives_everything", format!("{out}\n\n{}", transcript(&SessionDir::open(&session.location).unwrap())));
    assert_eq!(works.len(), 1, "one work session");
    assert_complete(&out);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn the_plan_gives_everything_a_work_session_needs() {
    let env = setup();
    let sd = plan(
        &env,
        SCAN,
        None,
    )
    .await;
    let llm = mixed(|_| None).await;
    look(&env, &sd, &llm).await;
    write("the_plan_gives_everything", format!("{}{}", todo_table(&sd), transcript(&sd)));
    let list = list_of(sd.path());
    assert!(!list.todos.is_empty());
    let all: String = list.todos.iter().map(|t| format!("{}\n{:?}\n", t.content, t.accept)).collect();
    assert_complete(&all);
    assert!(verifications(&list).is_empty(), "a scan shows its result itself: {:?}", verifications(&list));
}
