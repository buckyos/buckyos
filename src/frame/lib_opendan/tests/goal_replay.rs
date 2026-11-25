//! Real-model samples of Goals (notepads/opendan-goal-session-improve-todo.md):
//! what the prompts and skills are to make of the host's mechanics, which
//! unit tests cover with scripted models. Hours pass on a hand-moved clock;
//! the transcript of every case goes to `target/goal-replay/<case>.md` and is
//! to be read.
//!
//! ```text
//! cargo test -p libopendan --test goal_replay -- --ignored --nocapture --test-threads=1 [case]
//! ```

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::plan::*;
use common::replay::*;
use common::*;
use libopendan::host::goals::{GoalClock, GoalHost, GoalHostOptions, ManualClock};
use libopendan::host::Supervisor;
use libopendan::notify::{LocalNotify, NotifyAnswer, NotifyHost, NotifyKind, NotifyListReq};
use libopendan::protocol::*;
use libopendan::state::{AgentStateClient, GoalCaller, GoalCreate};
use libopendan::SessionDir;
use serde_json::json;

struct Hosted {
    sup: Arc<Supervisor>,
    clock: Arc<ManualClock>,
    agent: Arc<libopendan::FsAgentStateClient>,
    notify: Arc<LocalNotify>,
}

fn goal_env() -> Env {
    let env = setup();
    for dir in ["goal_skills", "goal_checks"] {
        copy_dir(&Path::new(PACKAGE).join(dir), &env.agent_root.join(dir));
    }
    // The built-in Goals stay out of these cases.
    let toml = env.agent_root.join("agent.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(&toml, text.replace("[session.goal.self_improve]\nenabled = true", "[session.goal.self_improve]\nenabled = false")).unwrap();
    env
}

fn hosted(env: &Env, llm: Arc<Mixed>, works: usize) -> Hosted {
    let clock = ManualClock::at(libopendan::now_ms());
    let mut opts = GoalHostOptions::new(env.agent_root.join("sessions"), real_context());
    opts.clock = clock.clone();
    opts.budget_every = Duration::ZERO;
    opts.answers_every = Duration::ZERO;
    let notify = Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap());
    let mut h = host_of(env, llm);
    h.max_child_concurrency = works;
    h.notify = Some(notify.clone());
    h.options.goal_clock = GoalClock(clock.clone());
    Hosted {
        sup: Supervisor::new(h, None).with_goals(GoalHost::new(opts)),
        clock,
        agent: env.agent(),
        notify,
    }
}

impl Hosted {
    async fn tick(&self) {
        self.sup.tick().await.unwrap();
        self.sup.tick_goals().await.unwrap();
    }

    async fn until(&self, what: &str, secs: u64, f: impl AsyncFn(&Hosted) -> bool) -> bool {
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            self.tick().await;
            if f(self).await {
                return true;
            }
            if std::time::Instant::now() > end {
                eprintln!("timed out waiting for {what}");
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn activations(&self, id: &str) -> Vec<GoalActivation> {
        self.agent.goals().activations(id, &Default::default()).await.unwrap()
    }

    async fn session(&self, id: &str) -> SessionDir {
        let g = self.agent.goals().get(id).await.unwrap().unwrap();
        SessionDir::open(&self.agent.sessions().lookup(&g.session.sid).await.unwrap().unwrap().location).unwrap()
    }

    /// The next Activation after `n`, the clock moved by `step` seconds at a
    /// time while nothing runs.
    async fn next(&self, id: &str, n: usize, step: u64, steps: usize) -> bool {
        for _ in 0..steps {
            self.clock.advance(step * 1000);
            for _ in 0..6 {
                self.tick().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let open = self.session(id).await.state().unwrap().open_turn.is_some();
            let works_run = self
                .agent
                .sessions()
                .query(&RegistryQuery { goal_id: Some(id.into()), ..Default::default() })
                .await
                .unwrap()
                .iter()
                .any(|e| e.kind == SessionKind::Work && e.status.run_state != RunState::Finished);
            if open || works_run {
                self.until("activation or work", 600, async |h| {
                    let open = h.session(id).await.state().unwrap().open_turn.is_some();
                    let running = h
                        .agent
                        .sessions()
                        .query(&RegistryQuery { goal_id: Some(id.into()), ..Default::default() })
                        .await
                        .unwrap()
                        .iter()
                        .any(|e| e.kind == SessionKind::Work && e.status.run_state != RunState::Finished);
                    !open && !running
                })
                .await;
            }
            if self.activations(id).await.len() > n {
                return true;
            }
        }
        false
    }

    async fn goals(&self) -> Vec<GoalDefinition> {
        self.agent.goals().list().await.unwrap()
    }
}

fn summary_of(acts: &[GoalActivation]) -> String {
    let mut out = String::from("## Activations\n\n");
    for a in acts {
        out.push_str(&format!(
            "- {} {} / {} — {} tokens, {} runs; reasons {:?}; works {:?}\n  {}\n",
            a.seq,
            a.outcome.as_str(),
            a.decision.as_str(),
            a.cost.total_tokens,
            a.cost.runs,
            a.reasons.iter().map(|r| (libopendan::goal::event_name(r.kind), r.note.clone().unwrap_or_default())).collect::<Vec<_>>(),
            a.works_created,
            a.summary.replace('\n', " ")
        ));
    }
    out.push('\n');
    out
}

fn write(case: &str, text: String) {
    write_to("goal-replay", case, text);
}

/// Characters of the parts of each Goal request.
#[derive(Default, Clone)]
struct Parts {
    system: usize,
    header: usize,
    before: usize,
    after: usize,
}

fn parts_of(req: &llm_context::deps::LlmInferenceRequest) -> Parts {
    let mut p = Parts::default();
    let at = req
        .messages
        .iter()
        .rposition(|m| m.role == buckyos_api::AiRole::User && m.text_content().starts_with("<goal_activation"))
        .unwrap_or(0);
    for (i, m) in req.messages.iter().enumerate() {
        let n = render(std::slice::from_ref(m)).len();
        match m.role {
            buckyos_api::AiRole::System => p.system += n,
            _ if i == at => p.header += n,
            _ if i < at => p.before += n,
            _ => p.after += n,
        }
    }
    p
}

// ---------------------------------------------------------------------------
// S0-1: what an Activation costs, fresh and with the history
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn cost_of_an_activation() {
    let mut out = String::new();
    for context in [GoalContext::Fresh, GoalContext::Carry] {
        let env = goal_env();
        let seen: Arc<Mutex<Vec<Parts>>> = Default::default();
        let log = seen.clone();
        let llm = mixed(move |req| {
            log.lock().unwrap().push(parts_of(req));
            None
        })
        .await;
        let h = hosted(&env, llm, 0);
        let inbox = env.root.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut req = GoalCreate {
            objective: format!("留意目录 {}：出现新文件时通知主人文件名。目前不要改检查脚本和时间表。", inbox.display()),
            title: Some("收件箱".into()),
            healthy_state: Some("每个新文件主人都收到了通知".into()),
            ..Default::default()
        };
        req.scope.paths = vec![inbox.display().to_string()];
        req.schedule.every_secs = Some(3600);
        req.schedule.min_interval_secs = Some(0);
        req.schedule.context = Some(context);
        let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
        h.tick().await;
        for n in 0..5 {
            h.next(&g.goal_id, n, 3600, 2).await;
        }
        let acts = h.activations(&g.goal_id).await;
        let usage = h.session(&g.goal_id).await.usage().unwrap();
        out.push_str(&format!("# context {}\n\n{}", context.as_str(), summary_of(&acts)));
        for (p, u) in seen.lock().unwrap().iter().zip(&usage) {
            out.push_str(&format!(
                "- system {} header {} history {} this activation {} chars; input {} output {} tokens ({})\n",
                p.system, p.header, p.before, p.after, u.input_tokens, u.output_tokens, u.model.clone()
            ));
        }
        out.push('\n');
        h.sup.shutdown().await;
    }
    write("cost_of_an_activation", out);
}

// ---------------------------------------------------------------------------
// CX3 / V1: a Goal woken for nothing makes its own check stricter
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn an_idle_goal_tightens_its_check() {
    let env = goal_env();
    let llm = mixed(|_| None).await;
    let h = hosted(&env, llm, 0);
    let inbox = env.root.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join("old.txt"), "already handled").unwrap();
    let mut req = GoalCreate {
        objective: format!("{} 里出现新文件时通知主人，说明文件名。old.txt 已经处理过。", inbox.display()),
        title: Some("收件箱".into()),
        healthy_state: Some("每个新文件主人都收到了通知，只通知一次".into()),
        ..Default::default()
    };
    req.scope.paths = vec![inbox.display().to_string()];
    req.schedule.every_secs = Some(300);
    req.schedule.min_interval_secs = Some(0);
    // Passes every time: what a script written in a hurry does.
    req.check_script = Some("export default function check(ctx, state) { return { run: true, reason: 'time to look' }; }\n".into());
    let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
    h.tick().await;
    let mut changed = None;
    for n in 0..6 {
        if !h.next(&g.goal_id, n, 300, 4).await {
            break;
        }
        let now = h.agent.goals().get(&g.goal_id).await.unwrap().unwrap();
        if now.schedule.check.as_ref().is_some_and(|c| c.version > 1) {
            changed = Some(n + 1);
            break;
        }
    }
    let script = h.agent.goals().check_script(&g.goal_id, None).await.unwrap().unwrap_or_default();
    // From here on an unchanged inbox wakes nobody; a new file does.
    let before = h.activations(&g.goal_id).await.len();
    for _ in 0..6 {
        h.clock.advance(3_600_000);
        for _ in 0..4 {
            h.tick().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let quiet = h.activations(&g.goal_id).await.len() - before;
    std::fs::write(inbox.join("invoice.pdf"), "new").unwrap();
    let woke = h.next(&g.goal_id, before + quiet, 3600, 4).await;
    let acts = h.activations(&g.goal_id).await;
    let sent: Vec<_> = h.notify.list(NotifyListReq::default()).await.unwrap().into_iter().filter_map(|v| v.prompt.title).collect();
    let changes = h.agent.goals().changes(&g.goal_id, 0).await.unwrap();
    write(
        "an_idle_goal_tightens_its_check",
        format!(
            "changed in activation {changed:?}; woken by the unchanged inbox afterwards: {quiet}; by a new file: {woke}; notifications {sent:?}\n\n{}```ts\n{script}\n```\n\n{}\n{}",
            summary_of(&acts),
            changes.iter().map(|c| format!("- {} {} {:?} {}\n", c.kind, c.via, c.reason, c.detail)).collect::<String>(),
            transcript(&h.session(&g.goal_id).await)
        ),
    );
    h.sup.shutdown().await;
    assert!(changed.is_some_and(|n| n <= 4), "{changed:?}");
    assert!(quiet <= 1, "an unchanged inbox wakes it {quiet} times");
    assert!(woke || sent.iter().any(|t| t.contains("invoice")), "{sent:?}");
}

// ---------------------------------------------------------------------------
// FH3 / V2: what does not work is not tried again and again
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn failing_work_ends_in_a_question() {
    let env = goal_env();
    let llm = mixed(|req| {
        let s = system_of(req);
        (!s.contains("You are the context of one long-lived goal")).then(|| {
            tool_call(
                "w1",
                "report",
                json!({ "intent": "The task cannot be done.", "report": "失败：备份服务器 198.51.100.7 拒绝了登录（Permission denied (publickey)），换 sftp 和 rsync 都是同样的结果。我没有这台机器的密钥，也找不到别的账号。", "is_end": true, "outcome": "failed" }),
            )
        })
    })
    .await;
    let h = hosted(&env, llm, 4);
    let photos = env.root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::write(photos.join("a.jpg"), "a").unwrap();
    let mut req = GoalCreate {
        objective: format!("每天把 {} 备份到备份服务器 198.51.100.7（用户 backup）。连接和备份都由 work 去做：能不能连上由 work 在它的环境里判断，激活里不要自己连那台机器，也不要因为自己连不上就停。", photos.display()),
        title: Some("照片备份".into()),
        healthy_state: Some("当天的备份成功完成".into()),
        ..Default::default()
    };
    req.schedule.every_secs = Some(86_400);
    req.schedule.min_interval_secs = Some(0);
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
    h.tick().await;
    let mut asked = None;
    for n in 0..8 {
        if !h.next(&g.goal_id, n, 1800, 60).await {
            break;
        }
        let st = h.agent.goals().state(&g.goal_id).await.unwrap().unwrap();
        if asked.is_none() && !st.questions.is_empty() {
            asked = Some(n + 1);
        }
        if asked.is_some_and(|a| n + 1 >= a + 2) {
            break;
        }
    }
    let acts = h.activations(&g.goal_id).await;
    let works = works_of(&env, None).await;
    let st = h.agent.goals().state(&g.goal_id).await.unwrap().unwrap();
    let sent = h.notify.list(NotifyListReq::default()).await.unwrap();
    write(
        "failing_work_ends_in_a_question",
        format!(
            "asked in activation {asked:?}; {} works; questions {:#?}\n\nnotifications: {:#?}\n\n{}{}\n{}",
            works.len(),
            st.questions,
            sent.iter().map(|v| (v.kind, v.prompt.title.clone(), v.prompt.message.clone())).collect::<Vec<_>>(),
            summary_of(&acts),
            works.iter().map(|w| format!("- {}\n", w.config().unwrap().session.objective)).collect::<String>(),
            transcript(&h.session(&g.goal_id).await)
        ),
    );
    h.sup.shutdown().await;
    assert!(asked.is_some(), "the user is asked");
    assert!(works.len() <= 4, "{} works", works.len());
    assert!(sent.iter().any(|v| v.kind == NotifyKind::Input), "the question reaches the user");
}

// ---------------------------------------------------------------------------
// FH5 / V3: a broken check script is repaired when it wakes the Goal
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_broken_check_is_repaired() {
    let env = goal_env();
    let llm = mixed(|_| None).await;
    let h = hosted(&env, llm, 0);
    let mut req = GoalCreate {
        objective: "提醒主人：工作日 15:00、15:50、16:40、17:30（Asia/Shanghai）站起来活动 10 分钟。".into(),
        title: Some("站立提醒".into()),
        healthy_state: Some("每个提醒都按时送到".into()),
        ..Default::default()
    };
    req.schedule.cron = Some("*/10 15-17 * * 1-5".into());
    req.schedule.timezone = Some("Asia/Shanghai".into());
    req.check_script = Some("export default function check(ctx, state) {\n  const now = local(ctx.now_ms, ctx.timezone);\n  return { run: false, notify: { title: '站起来' } };\n}\n".into());
    let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
    h.tick().await;
    // 14:55 in Shanghai on the next Monday, then every ten minutes.
    let day = 86_400_000;
    let mut at = (libopendan::now_ms() / day + 1) * day + (6 * 60 + 55) * 60_000;
    while (at / day + 4) % 7 != 1 {
        at += day;
    }
    h.clock.set(at);
    h.tick().await;
    let mut woke = None;
    for slot in 1..=8 {
        if h.next(&g.goal_id, 0, 600, 1).await {
            woke = Some(slot);
            break;
        }
    }
    let script = h.agent.goals().check_script(&g.goal_id, None).await.unwrap().unwrap_or_default();
    let mut sent = Vec::new();
    for _ in 0..20 {
        h.clock.advance(600_000);
        for _ in 0..4 {
            h.tick().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        sent = h.notify.list(NotifyListReq::default()).await.unwrap().into_iter().filter_map(|v| v.prompt.title).collect();
    }
    let schedule = h.agent.goals().schedule(&g.goal_id).await.unwrap().unwrap();
    let acts = h.activations(&g.goal_id).await;
    write(
        "a_broken_check_is_repaired",
        format!(
            "woken after slot {woke:?}; sent afterwards {sent:?}; errors {} last {:?}\n\n{}```ts\n{script}\n```\n\n{}",
            schedule.check_errors,
            schedule.last_check,
            summary_of(&acts),
            transcript(&h.session(&g.goal_id).await)
        ),
    );
    h.sup.shutdown().await;
    assert!(woke.is_some_and(|s| s <= 4), "{woke:?}");
    assert!(script.contains("slot_ms"), "{script}");
    assert!(!sent.is_empty(), "the rest of the day's reminders are sent");
    assert_eq!(acts.len(), 1, "no inference for a reminder");
}

// ---------------------------------------------------------------------------
// FX4 / V4 / V5 / V8: what the conversation makes of a request
// ---------------------------------------------------------------------------

struct Chat {
    env: Env,
    sd: SessionDir,
    llm: Arc<Mixed>,
    asked: Arc<Mutex<String>>,
}

impl Chat {
    async fn new() -> Self {
        Self::on(goal_env()).await
    }

    async fn on(env: Env) -> Self {
        // The calendar: what it was asked is kept, nothing is written.
        let xcall = env.agent_root.join("tools/xcall");
        std::fs::write(
            &xcall,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$OPENDAN_AGENT_ROOT/xcall.txt\"\necho '{\"agent_tool_protocol\":\"1\",\"status\":\"success\",\"cmd_name\":\"xcall\",\"summary\":\"done\",\"detail\":{\"result\":{\"record_ref\":\"obj://cal/1\",\"open_url\":\"https://test.example/calendar/1\"}}}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&xcall, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let template = libopendan::SessionTemplate::load("ui", Some(&env.agent_root)).unwrap();
        let mut spec = template.spec(template.objective_or_default(false));
        spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Aalice"));
        spec.owner = Some(OWNER.into());
        spec.outbound = Some(OutboundBinding { to: OWNER.into(), to_session: None, kind: "chat".into() });
        spec.timezone = Some("Asia/Shanghai".into());
        spec.prompt.llm_context = real_context();
        spec.freeze = true;
        let sd = env.create_work(spec).await;
        // The conversation hands every request to intent analysis (whether it
        // would is not the case here) and only the analysis is the model's.
        let asked: Arc<Mutex<String>> = Default::default();
        let words = asked.clone();
        let llm = mixed(move |req| {
            let analysis = user_texts(req).iter().any(|t| t.contains("<intent_analysis>"));
            if analysis {
                return None;
            }
            Some(if has_tool_result(req, "ti1").is_none() {
                tool_call("ti1", "try_analyze_user_intent", json!({ "intent": "The request needs work.", "request": *words.lock().unwrap() }))
            } else {
                text(&json!({ "reply": "好的", "topic": null, "artifacts": [] }).to_string())
            })
        })
        .await;
        Self { env, sd, llm, asked }
    }

    async fn say(&self, words: &str) {
        *self.asked.lock().unwrap() = words.to_string();
        say(&self.env, &self.sd, &self.llm, words).await;
    }

    async fn goals(&self) -> Vec<GoalDefinition> {
        self.env.agent().goals().list().await.unwrap()
    }

    async fn works(&self) -> Vec<SessionDir> {
        works_of(&self.env, Some(self.sd.sid())).await
    }

    fn xcalls(&self) -> String {
        std::fs::read_to_string(self.env.agent_root.join("xcall.txt")).unwrap_or_default()
    }

    async fn goal(&self, objective: &str, title: &str, cron: Option<&str>) -> GoalDefinition {
        let mut req = GoalCreate {
            objective: objective.into(),
            title: Some(title.into()),
            healthy_state: Some("一切如目标所述".into()),
            check_script: Some("export default () => false;\n".into()),
            ..Default::default()
        };
        req.schedule.every_secs = cron.is_none().then_some(3600);
        req.schedule.cron = cron.map(str::to_string);
        req.schedule.timezone = Some("Asia/Shanghai".into());
        let from = GoalCaller::from_session(APP, self.sd.sid());
        let g = self.env.agent().goals().create(&req, &from).await.unwrap();
        // Its Goal Session exists (the host made it), so it takes messages.
        libopendan::goal::ensure_goal_session(
            self.env.agent().as_ref(),
            &g,
            1,
            APP,
            self.env.channels().as_ref(),
            &self.env.agent_root.join("sessions"),
            &real_context(),
            "goal",
        )
        .await
        .unwrap();
        g
    }

    async fn record(&self, case: &str) -> String {
        let mut out = String::new();
        for g in self.goals().await {
            out.push_str(&format!(
                "## goal {} {}\n\nrevision {} status {:?} every {:?} cron {:?} {:?} start {} check {:?} via {:?}\n\n{}\n\n",
                g.goal_id, g.title, g.revision, g.status, g.schedule.every_secs, g.schedule.cron, g.schedule.timezone, g.schedule.start_on_create, g.schedule.check.map(|c| c.version), g.created_by, g.objective
            ));
        }
        for w in self.works().await {
            let cfg = w.config().unwrap();
            out.push_str(&format!(
                "## work {} class {} behavior {:?} manuals {:?} workspace {:?}\n\n{}\n\n{}\n\n",
                w.sid(), cfg.session.class, cfg.prompt.behavior, cfg.prompt.cmd_skills, cfg.workspace, cfg.session.objective, initial_text(&cfg)
            ));
        }
        out.push_str(&format!("## xcall\n\n{}\n\n{}", self.xcalls(), transcript(&self.sd)));
        write(case, out.clone());
        out
    }
}

fn initial_text(cfg: &SessionConfig) -> String {
    cfg.prompt
        .initial_inputs
        .iter()
        .map(|i| serde_json::to_string(i).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\\n", "\n")
}

/// V4: nothing to be made — the goal itself, at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn watching_something_is_a_goal_at_once() {
    let c = Chat::new().await;
    c.say("帮我盯着 https://198.51.100.20/releases.atom ，出新版本就告诉我版本号，每小时看一次就行。").await;
    c.record("watching_something_is_a_goal_at_once").await;
    assert_eq!(c.goals().await.len(), 1);
    assert!(c.works().await.is_empty());
}

/// V4: made first, then looked after — a work session whose task ends with
/// the goal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn made_then_looked_after_is_work_that_ends_with_a_goal() {
    let c = Chat::new().await;
    c.say("做一个状态页：每小时检查 https://198.51.100.21 和 https://198.51.100.22 能不能打开，结果写成一个静态网页，放在一个新的工作区里（就放你自己的工作区目录下，名字叫 status-page），以后一直维护它，挂了就通知我。不用再问我细节。").await;
    let out = c.record("made_then_looked_after").await;
    let works = c.works().await;
    assert_eq!(works.len(), 1, "one work session");
    assert!(c.goals().await.is_empty(), "the work sets the goal up, not the analysis");
    let cfg = works[0].config().unwrap();
    let task = format!("{}\n{}", cfg.session.objective, initial_text(&cfg));
    assert!(task.contains("goal") || task.contains("Goal"), "{task}");
    assert!(task.contains("goal_id"), "its report gives the goal: {task}");
    let manuals = if cfg.session.class == "plan_simple" { task.contains("goal_handover") } else { cfg.prompt.cmd_skills.iter().any(|s| s == "goal_handover") };
    assert!(manuals, "{out}");
}

/// V5: an appointment goes into the calendar, with its reminder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn an_appointment_goes_into_the_calendar() {
    let c = Chat::new().await;
    c.say("明天下午三点（上海时间）开评审会，开始前十分钟提醒我。").await;
    c.record("an_appointment_goes_into_the_calendar").await;
    assert!(c.goals().await.is_empty(), "no goal for it");
    let x = c.xcalls();
    assert!(x.contains("owner.cal add") && x.contains("reminders"), "{x}");
}

/// V5: a habit joins the goal that sends reminders, not the calendar.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_habit_joins_the_reminder_goal() {
    let c = Chat::new().await;
    let g = c.goal("按时提醒主人：工作日 09:00（Asia/Shanghai）“先写今天最重要的三件事”。提醒由检查脚本发，不推理。", "提醒", Some("0 9 * * 1-5")).await;
    c.say("工作日下午三点到六点，每 50 分钟提醒我站起来活动一下。").await;
    c.record("a_habit_joins_the_reminder_goal").await;
    let goals = c.goals().await;
    assert_eq!(goals.len(), 1, "the reminder goal takes it");
    assert!(goals[0].revision > g.revision, "its definition covers it");
    assert!(goals[0].objective.contains("09:00"), "what it did stays: {}", goals[0].objective);
    assert!(!c.xcalls().contains("owner.cal add"), "{}", c.xcalls());
    let posted = std::fs::read_dir(&c.env.queue_dir).is_ok();
    assert!(posted);
}

/// FX4: a request that belongs to a goal there is makes no new one; a small
/// change of it is made on the spot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_small_change_revises_the_goal() {
    let c = Chat::new().await;
    let g = c.goal("盯着 https://198.51.100.20/releases.atom 的新版本，出新版本就通知主人，任何时间都通知。", "新版本监视", None).await;
    c.say("新版本那个以后周末别提醒我，攒到周一一起说。").await;
    c.record("a_small_change_revises_the_goal").await;
    let goals = c.goals().await;
    assert_eq!(goals.len(), 1);
    assert!(c.works().await.is_empty(), "no work session for it");
    assert!(goals[0].revision > g.revision, "{:?}", goals[0]);
}

/// V8: with sixteen goals a seventeenth is made only when none can take it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_seventeenth_goal_needs_a_reason() {
    let c = Chat::new().await;
    for i in 1..=16 {
        c.goal(&format!("盯着 https://198.51.100.{i}/feed.xml，有新文章就通知主人标题。"), &format!("订阅 {i}"), None).await;
    }
    c.say("再帮我盯着 https://198.51.100.77/feed.xml ，有新文章告诉我标题。").await;
    let out = c.record("a_seventeenth_goal_needs_a_reason").await;
    let goals = c.goals().await;
    let revised = goals.iter().any(|g| g.revision > 1);
    let said = out.contains("16") || out.contains("十六");
    assert!(revised || (goals.len() == 17 && said), "folded into one, or a reason is given");
}

/// WG5: the user agrees to an upgrade a goal asked for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn an_agreed_upgrade_becomes_work() {
    let c = Chat::new().await;
    let dir = c.env.root.join("status-page");
    std::fs::create_dir_all(dir.join("upgrades")).unwrap();
    let ws = c.env.workspace(&dir).await;
    let note = dir.join("upgrades/2026-11-24-mail-alerts.md");
    let g = c.goal("维护 status-page 工作区的状态页；检查每小时一次。", "状态页", None).await;
    std::fs::write(
        &note,
        format!(
            "# 状态页：加一列响应时间\n\n发生了什么：主人问过两次各站的响应时间，页面上没有。\n影响：页面多一列，数据多存 24 小时。\n不升级：继续只显示能不能打开。\n\n## 任务书草稿\n目标：状态页显示每个站最近 24 小时的平均响应时间。\n范围：工作区 {}（{}），check.py 和 index.html 模板。\n完成的标准：页面上有这一列，数字和手工 curl 的结果同一量级。\n报告：改了哪些文件、页面截图路径。\n最后一步：改写 goal {}。\n",
            ws.workspace_id, dir.display(), g.goal_id
        ),
    )
    .unwrap();
    let notify = LocalNotify::enable(&c.env.agent_root, AGENT).unwrap();
    let mut req = libopendan::notify::NotifySendReq::notice(
        "",
        "状态页建议升级：加一列响应时间",
        format!("说明在 {}；goal {}", note.display(), g.goal_id),
        format!("upgrade:{}:latency", g.goal_id),
    );
    req.source = Some(libopendan::notify::NotifySource { goal_id: Some(g.goal_id.clone()), ..Default::default() });
    notify.send(req).await.unwrap();
    c.say(&format!("状态页那条升级建议我看了（{}），升级吧。", note.display())).await;
    let out = c.record("an_agreed_upgrade_becomes_work").await;
    let works = c.works().await;
    assert_eq!(works.len(), 1, "{out}");
    let cfg = works[0].config().unwrap();
    let task = format!("{}\n{}", cfg.session.objective, initial_text(&cfg));
    assert!(task.contains(&g.goal_id), "its last step names the goal: {task}");
    assert_eq!(cfg.workspace.map(|w| w.workspace_id), Some(ws.workspace_id));
}

// ---------------------------------------------------------------------------
// FX2: the work hands over
// ---------------------------------------------------------------------------

/// A conversation with the owner (its model is not asked anything).
async fn chat_of(env: &Env) -> SessionDir {
    let mut spec = libopendan::SessionTemplate::load("ui", Some(&env.agent_root)).unwrap().spec("chat");
    spec.owner = Some(OWNER.into());
    spec.outbound = Some(OutboundBinding { to: OWNER.into(), to_session: None, kind: "chat".into() });
    spec.prompt.llm_context = real_context();
    env.create_work(spec).await
}

async fn handover(case: &str, task: &str) -> (Hosted, Env, SessionDir, String) {
    let env = goal_env();
    let llm = mixed(|_| None).await;
    let ui = chat_of(&env).await;
    let dir = env.root.join("status-page");
    std::fs::create_dir_all(&dir).unwrap();
    let ws = env.workspace(&dir).await;
    // Its host runs along: what the work installs is checked and run.
    let h = hosted(&env, llm, 1);
    h.tick().await;
    let work = libopendan::api::create_sub_session(
        env.agent().as_ref(),
        APP,
        ui.sid(),
        libopendan::api::SubSessionSpec {
            objective: task.replace("{workspace}", &dir.display().to_string()),
            title: Some("状态页".into()),
            msgs: vec![task.replace("{workspace}", &dir.display().to_string())],
            behavior: Some("work_simple".into()),
            cmd_skills: Some(vec!["goal_handover".into()]),
            key: Some("handover".into()),
            workspace: libopendan::api::SubWorkspace::Id(ws.workspace_id.clone()),
            ..Default::default()
        },
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    h.until("the work", 900, async |_| work.state().unwrap().outcome.is_some()).await;
    for _ in 0..10 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut out = format!("report:\n\n{}\n\n", std::fs::read_to_string(work.path().join("report.md")).unwrap_or_default());
    for g in h.goals().await {
        let s = h.agent.goals().schedule(&g.goal_id).await.unwrap();
        out.push_str(&format!(
            "## goal\n\n{}\n\ncheck: {:?}\nactivations: {}\n\n```ts\n{}\n```\n\n",
            serde_json::to_string_pretty(&g).unwrap(),
            s.and_then(|s| s.last_check),
            h.activations(&g.goal_id).await.len(),
            h.agent.goals().check_script(&g.goal_id, None).await.unwrap().unwrap_or_default()
        ));
    }
    out.push_str(&transcript(&work));
    write(case, out.clone());
    (h, env, work, out)
}

/// V4: the Work ends with the Goal; the Goal starts with a check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn the_work_hands_over_to_a_goal() {
    let (h, _env, work, out) = handover(
        "the_work_hands_over_to_a_goal",
        "目标：在工作区 {workspace} 里做一个磁盘用量的记录：脚本 `record.sh` 把 `df -P /` 的用量百分比连同时间追加到 `usage.csv`，先手工跑一次。范围：只动这个工作区。完成的标准：usage.csv 有一行合法数据；负责维护它的 goal 已经建好，从检查开始，检查跑过。之后由 goal 维护：每小时看一次 usage.csv 最后一行，百分比到 90 以上就通知主人；文件超过两小时没有更新也通知。报告：文件路径，`result` 里给 `goal_id`。",
    )
    .await;
    let goals = h.goals().await;
    assert_eq!(goals.len(), 1, "{out}");
    let g = &goals[0];
    assert_eq!(g.created_by.work.as_deref(), Some(work.sid()));
    assert!(g.schedule.check.is_some() && !g.schedule.start_on_create, "{out}");
    assert!(h.activations(&g.goal_id).await.is_empty(), "no inference at the start");
    assert!(h.agent.goals().schedule(&g.goal_id).await.unwrap().unwrap().last_check.is_some(), "its first run was a check");
    assert!(work.state().unwrap().result.is_some_and(|r| serde_json::to_string(&r).unwrap().contains(&g.goal_id)), "{out}");
    h.sup.shutdown().await;
}

/// V4: the Work cannot do it: it says so, and no goal stays behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn failed_work_leaves_no_goal() {
    let (h, _env, work, out) = handover(
        "failed_work_leaves_no_goal",
        "目标：在工作区 {workspace} 里用命令 `diskpulse`（本机已经装好）做一个磁盘心跳记录脚本 `pulse.sh`，它把 `diskpulse --json` 的输出追加到 pulse.jsonl。范围：只动这个工作区，只能用 diskpulse，不要用别的命令代替，也不要安装任何东西。完成的标准：pulse.jsonl 有一行 diskpulse 的真实输出；负责维护它的 goal 已经建好，从检查开始。报告：文件路径，`result` 里给 `goal_id`。",
    )
    .await;
    let live: Vec<_> = h.goals().await.into_iter().filter(|g| g.status.is_live()).collect();
    assert!(live.is_empty(), "{out}");
    assert_eq!(work.state().unwrap().outcome, Some(Outcome::Failed), "{out}");
    h.sup.shutdown().await;
}

/// V10 (without a Zone): the Work builds, installs the job and hands over;
/// the routine runs without an inference; a fault wakes the Goal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_job_runs_what_was_built_and_its_failure_wakes_the_goal() {
    let case = "a_job_runs_what_was_built_and_its_failure_wakes_the_goal";
    let (h, env, _work, mut out) = handover(
        case,
        "目标：在工作区 {workspace} 里做一个待办统计：程序 `summary.sh` 读工作区里的 `tasks.txt`（每行以 `[ ]` 或 `[x]` 开头，先放五行样例），把完成和未完成的条数写成工作区里的 `summary.json`（`{\"done\":…,\"open\":…,\"at\":…}`），读不到 tasks.txt 时报错退出。先手工跑通。它以后每小时自动跑一次，例行运行不要用推理：装成负责维护它的 goal 的作业。范围：只动这个工作区。完成的标准：summary.json 的数字对；goal 已经建好，从检查开始；作业装上了；检查脚本只看作业的运行记录，连续失败两次时叫醒 goal，不看文件的时间。维护范围：程序坏了或者输入文件换了名字，由 goal 自己找到原因修好，用户看到的 summary.json 不变。报告：文件路径，`result` 里给 `goal_id` 和 `job_id`。",
    )
    .await;
    let goals = h.goals().await;
    assert_eq!(goals.len(), 1, "{out}");
    let id = goals[0].goal_id.clone();
    let jobs = async |h: &Hosted| h.agent.goals().jobs(&id).await.unwrap();
    h.until("the first run", 60, async |h| jobs(h).await.iter().any(|j| j.last_run.is_some())).await;
    for _ in 0..3 {
        h.next(&id, 0, 3600, 1).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let routine = jobs(&h).await;
    let quiet = h.activations(&id).await.len();
    let dir = env.root.join("status-page");
    let tasks = dir.join("tasks.txt");
    std::fs::rename(&tasks, dir.join("todo-list.txt")).expect("tasks.txt");
    let woken = h.next(&id, quiet, 3600, 6).await
        || h.until("the activation", 300, async |h| h.activations(&id).await.len() > quiet).await;
    for _ in 0..3 {
        h.clock.advance(3600 * 1000);
        for _ in 0..20 {
            h.tick().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let after = jobs(&h).await;
    let acts = h.activations(&id).await;
    let st = h.agent.goals().state(&id).await.unwrap().unwrap();
    out.push_str(&format!(
        "\n# jobs\n\nroutine {routine:#?}\n\nafter {after:#?}\n\nschedule {:#?}\n\nholding {:?}\n\nquestions {:#?}\n\n{}{}",
        h.agent.goals().schedule(&id).await.unwrap(),
        st.holding,
        st.questions,
        summary_of(&acts),
        transcript(&h.session(&id).await)
    ));
    write(case, out);
    h.sup.shutdown().await;
    assert!(!routine.is_empty() && routine.iter().all(|j| j.failures == 0 && j.last_run.is_some()), "{routine:#?}");
    assert_eq!(quiet, 0, "the routine infers nothing");
    assert!(woken, "the failing job wakes the goal");
    assert!(after.iter().all(|j| j.failures == 0) || !st.questions.is_empty(), "repaired, or the owner is asked: {after:#?}");
}

// ---------------------------------------------------------------------------
// WG4: a Goal operates, it does not develop
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn more_than_operation_is_asked_for() {
    let env = goal_env();
    let llm = mixed(|_| None).await;
    let h = hosted(&env, llm, 0);
    let dir = env.root.join("status-page");
    std::fs::create_dir_all(&dir).unwrap();
    let ws = env.workspace(&dir).await;
    // The site answers every request with a redirect to its login page.
    let site = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", site.local_addr().unwrap().port());
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut c, _)) = site.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = c.read(&mut buf).await.unwrap_or(0);
                let answer = if buf[..n].starts_with(b"GET /login") || buf[..n].starts_with(b"HEAD /login") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nplease login"
                } else {
                    "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                let _ = c.write_all(answer.as_bytes()).await;
            });
        }
    });
    std::fs::write(dir.join("sites.json"), format!("[\"{url}\"]\n")).unwrap();
    std::fs::write(
        dir.join("check.sh"),
        "#!/bin/bash\n# 每个站一行：<时间> <地址> <状态码>\ncd \"$(dirname \"$0\")\"\nfor u in $(tr -d '[]\",' < sites.json); do\n  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 \"$u\")\n  case \"$code\" in 2*|3*) ok=true;; *) ok=false;; esac\n  printf '{\"at\":\"%s\",\"url\":\"%s\",\"code\":\"%s\",\"ok\":%s}\\n' \"$(date -Is)\" \"$u\" \"$code\" \"$ok\"\ndone > status.json\n",
    )
    .unwrap();
    std::fs::write(dir.join("status.json"), format!("{{\"at\":\"2026-11-25T09:00:00+08:00\",\"url\":\"{url}\",\"code\":\"302\",\"ok\":true}}\n")).unwrap();
    std::fs::write(dir.join("operations.md"), "# 状态页\n\n- `sites.json` 是检查的站点；`check.sh`（`bash check.sh`）写 `status.json`。\n- check.sh 只用 curl 看能不能打开，3xx 也算打开了。\n- 主人 2026-11-20 说：只要知道能不能打开就行，不用登录，账号不要放在这里。\n- 判断方法变了、要用到新的权限，属于升级。\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "2026-11-23 站方的运维通知：从 2026-11-25 起首页要登录才能看，未登录一律 302 到登录页，站挂了登录页也照样能打开；要账号登录进去才能判断站点是不是真的正常。\n").unwrap();
    let mut req = GoalCreate {
        objective: format!("维护工作区 {} 的状态页：每个站能不能打开要准。做法见 operations.md，站方的通知在 notes.txt。", dir.display()),
        title: Some("状态页".into()),
        healthy_state: Some("status.json 如实反映每个站能不能打开".into()),
        ..Default::default()
    };
    req.scope.workspace = Some(ws.workspace_id.clone());
    req.schedule.min_interval_secs = Some(0);
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
    h.until("the activation", 600, async |h| !h.activations(&g.goal_id).await.is_empty()).await;
    let acts = h.activations(&g.goal_id).await;
    let notes: Vec<String> = std::fs::read_dir(dir.join("upgrades"))
        .map(|d| d.flatten().map(|e| format!("{:?}\n{}", e.file_name(), std::fs::read_to_string(e.path()).unwrap_or_default())).collect())
        .unwrap_or_default();
    let sent = h.notify.list(NotifyListReq::default()).await.unwrap();
    let st = h.agent.goals().state(&g.goal_id).await.unwrap().unwrap();
    write(
        "more_than_operation_is_asked_for",
        format!(
            "{}notes {notes:#?}\n\nnotifications {:#?}\n\nquestions {:#?}\n\n{}",
            summary_of(&acts),
            sent.iter().map(|v| (v.prompt.title.clone(), v.prompt.message.clone())).collect::<Vec<_>>(),
            st.questions,
            transcript(&h.session(&g.goal_id).await)
        ),
    );
    h.sup.shutdown().await;
    assert!(!notes.is_empty(), "an upgrade note");
    assert!(sent.iter().any(|v| v.prompt.message.as_deref().unwrap_or_default().contains("upgrades/")), "{sent:#?}");
    assert!(works_of(&env, None).await.is_empty(), "it does not send work to change the page");
}

/// FH4: choices of a question, and what the answer does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_question_offers_choices_and_its_answer_is_followed() {
    let env = goal_env();
    let llm = mixed(|_| None).await;
    let h = hosted(&env, llm, 0);
    let dir = env.root.join("photos");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..3 {
        std::fs::write(dir.join(format!("dup-{i}.jpg")), "same").unwrap();
    }
    let mut req = GoalCreate {
        objective: format!("保持 {} 里没有重复的照片。删哪一份、还是移到 trash/，要先问过主人。", dir.display()),
        title: Some("照片去重".into()),
        healthy_state: Some("没有内容相同的照片".into()),
        ..Default::default()
    };
    req.scope.paths = vec![dir.display().to_string()];
    req.schedule.min_interval_secs = Some(0);
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &GoalCaller::owner(OWNER)).await.unwrap();
    h.until("the question", 600, async |h| !h.activations(&g.goal_id).await.is_empty()).await;
    let asked = h.notify.list(NotifyListReq::default()).await.unwrap().into_iter().find(|v| v.kind == NotifyKind::Input);
    let options = asked.as_ref().map(|v| libopendan::notify::prompt_options(&v.prompt)).unwrap_or_default();
    if let (Some(v), Some(o)) = (&asked, options.first()) {
        h.notify.answer(&v.notify_id, NotifyAnswer::option(o.value.clone()), None).await.unwrap();
        h.until("the answer", 600, async |h| h.activations(&g.goal_id).await.len() >= 2).await;
    }
    let acts = h.activations(&g.goal_id).await;
    write(
        "a_question_offers_choices",
        format!("{asked:#?}\n\n{}{}", summary_of(&acts), transcript(&h.session(&g.goal_id).await)),
    );
    h.sup.shutdown().await;
    assert!(options.len() >= 2, "{asked:#?}");
    assert_eq!(acts.len(), 2);
}

// ---------------------------------------------------------------------------
// SI4: a command two works lacked becomes a tool the third one uses
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real model through the local Zone's AICC"]
async fn a_missing_command_becomes_a_tool() {
    let env = goal_env();
    let toml = env.agent_root.join("agent.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(&toml, text.replace("[session.goal.self_improve]\nenabled = false", "[session.goal.self_improve]\nenabled = true")).unwrap();
    std::fs::create_dir_all(env.agent_root.join(".meta")).unwrap();
    std::fs::write(env.agent_root.join(".meta/skills_state.json"), json!({ "self_improve": { "memory": false, "manuals": false } }).to_string()).unwrap();
    let llm = mixed(|_| None).await;
    let data = env.root.join("exports");
    std::fs::create_dir_all(&data).unwrap();
    // A fixed-width export: a header line with the columns' widths, then the rows.
    for (name, rows) in [("a", 40), ("b", 55), ("c", 70)] {
        let mut t = String::from("#FWX1 date:10 account:8 amount:10\n");
        for i in 0..rows {
            t.push_str(&format!("2026-11-{:02}{:<8}{:>10}\n", i % 28 + 1, ["food", "rent", "travel"][i % 3], format!("{}.{:02}", i * 7, i % 100)));
        }
        std::fs::write(data.join(format!("{name}.fwx")), t).unwrap();
    }
    let task = |name: &str| {
        format!(
            "目标：把账目导出 {d}/{name}.fwx 按 account 汇总 amount，结果写成 {d}/{name}.json（account 到合计，两位小数）。FWX 是定宽文本：第一行是 `#FWX1 列:宽度 …`，后面每行一条记录。先跑 `ledgersum <文件>` 汇总；跑不了就想别的办法，别装软件。范围：只读 {name}.fwx，只写 {name}.json。完成的标准：合计和自己另算一遍的结果一致。报告：{name}.json 的内容。",
            d = data.display()
        )
    };
    let mut rounds = Vec::new();
    let mut out = String::new();
    let ui = chat_of(&env).await;
    for name in ["a", "b"] {
        let work = libopendan::api::create_sub_session(
            env.agent().as_ref(),
            APP,
            ui.sid(),
            libopendan::api::SubSessionSpec {
                objective: format!("汇总 {name}.fwx"),
                title: Some(format!("汇总 {name}.fwx")),
                msgs: vec![task(name)],
                behavior: Some("work_simple".into()),
                key: Some(format!("sum:{name}")),
                ..Default::default()
            },
            env.channels().as_ref(),
        )
        .await
        .unwrap();
        finish(&env, &work, &llm).await;
        rounds.push(calls_of(&work));
        out.push_str(&transcript(&work));
    }
    let h = hosted(&env, llm.clone(), 2);
    h.until("the goal", 60, async |h| !h.goals().await.is_empty()).await;
    let g = h.goals().await.remove(0);
    let woken = async |h: &Hosted| {
        let checked = h.agent.goals().schedule(&g.goal_id).await.unwrap().and_then(|s| s.last_check);
        checked.is_some_and(|c| c.outcome != CheckOutcome::True) || !h.activations(&g.goal_id).await.is_empty()
    };
    h.until("the activation", 900, woken).await;
    assert!(!h.activations(&g.goal_id).await.is_empty(), "the check names the command:\n{out}");
    for _ in 0..3 {
        if !new_manuals(&env).is_empty() && !h.notify.list(NotifyListReq::default()).await.unwrap().is_empty() {
            break;
        }
        let n = h.activations(&g.goal_id).await.len();
        h.next(&g.goal_id, n, 1800, 8).await;
    }
    let acts = h.activations(&g.goal_id).await;
    let sent: Vec<_> = h.notify.list(NotifyListReq::default()).await.unwrap().iter().map(|v| (v.prompt.title.clone(), v.prompt.message.clone())).collect();
    let manuals = new_manuals(&env);
    out.push_str(&format!("\n# self improvement\n\n{}manuals {manuals:#?}\n\nnotifications {sent:#?}\n\n{}", summary_of(&acts), transcript(&h.session(&g.goal_id).await)));
    h.sup.shutdown().await;
    // The third request goes through intent analysis.
    let c = Chat::on(env).await;
    let before: Vec<String> = c.works().await.iter().map(|w| w.sid().to_string()).collect();
    c.say(&format!("开一个工作去做：把账目导出 {d}/c.fwx（FWX 定宽文本）按 account 汇总 amount，写成 {d}/c.json（account 到合计，两位小数），和前两次一样。只读 c.fwx，只写 c.json。不用问我细节。", d = data.display())).await;
    let third = c.works().await.into_iter().find(|w| !before.contains(&w.sid().to_string()));
    out.push_str(&transcript(&c.sd));
    if let Some(w) = &third {
        finish(&c.env, w, &llm).await;
        rounds.push(calls_of(w));
        out.push_str(&format!("\nmanuals given: {:?}\n\n{}", w.config().unwrap().prompt.cmd_skills, transcript(w)));
    }
    out.push_str(&format!("\ncalls of the three works: {rounds:?}\n"));
    write("a_missing_command_becomes_a_tool", out);
    assert!(!manuals.is_empty(), "a manual of its own");
    assert!(!sent.is_empty(), "the owner was told");
    let third = third.expect("a work for the third request");
    let given = third.config().unwrap().prompt.cmd_skills;
    assert!(manuals.iter().any(|m| given.contains(m)), "given {given:?} of {manuals:?}");
    let text = std::fs::read_to_string(data.join("c.json")).expect("c.json");
    assert!(text.contains("travel"), "{text}");
    eprintln!("calls of the three works: {rounds:?}");
}

/// Tool calls of a work.
fn calls_of(sd: &SessionDir) -> usize {
    read_worklog(sd)
        .iter()
        .map(|e| match &e.body {
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls.len(),
            _ => 0,
        })
        .sum()
}

/// Names of the work manuals there are now and the package did not bring.
fn new_manuals(env: &Env) -> Vec<String> {
    libopendan::skills::list(&env.agent_root, Some(SkillKind::Work))
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .filter(|n| !Path::new(PACKAGE).join("cmd_skills").join(n).exists())
        .collect()
}
