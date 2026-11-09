//! Recall wiring (notepads/agent-memory-recall-wiring-todo.md R1, R2): the
//! batch-level recall and observation of a session through the Memory
//! facade, its forgetting, its state across restarts, and the runner that
//! shows the material to the model.
//!
//! The first half drives the facade directly (manual clock); the second
//! drives real UI and Work sessions with a scripted model.

mod common;
#[path = "../examples/support/memory_sim.rs"]
mod memory_sim;

use std::collections::BTreeSet;
use std::time::Duration;

use agent_tool::agent_memory::ObservationKind;
use common::*;
use libopendan::memory::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::SessionTemplate;
use memory_sim::{obs_op, observation, plan, scope, Host, ItemSpec, SimSession, SimSi};
use serde_json::json;

const BOB: &str = "did:bns:bob";
const CAROL: &str = "did:bns:carol";
const GROUP: &str = "did:bns:group-x";
const MIN: u64 = 60 * 1000;

fn host(tmp: &tempfile::TempDir) -> Host {
    Host::new(&tmp.path().join("agent"), "2026-11-09T09:00:00+08:00")
}

fn tags(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

/// One cognition (and its evidence) committed by the consolidation.
fn cognition(h: &mut Host, si: &SimSi, id: &str, statement: &str, tag: &[&str], subjects: &[&str]) {
    cognition_rev(h, si, id, statement, tag, subjects, None)
}

fn cognition_rev(
    h: &mut Host,
    si: &SimSi,
    id: &str,
    statement: &str,
    tag: &[&str],
    subjects: &[&str],
    expected: Option<u64>,
) {
    let ev = h.event(&format!("{id}/e{}", expected.unwrap_or(0)), "ui-src", ActorKind::User, statement);
    let obs = format!("obs_{}_{}", id.trim_start_matches("item_"), expected.unwrap_or(0));
    let p = plan(
        &format!("{id}@{}", expected.unwrap_or(0)),
        statement,
        vec![
            obs_op(&obs, ObservationKind::ExplicitStatement, statement, &ev, scope(subjects, &[])),
            ItemSpec {
                tags: tag,
                expected,
                ..ItemSpec::new(id, "preference", statement, &[obs.as_str()], scope(subjects, &[]), Basis::UserStatement)
            }
            .op(),
        ],
        Vec::new(),
    );
    let batch = Batch {
        id: format!("b-{id}"),
        materials: Vec::new(),
        context: Vec::new(),
        truncated: false,
        graph_seq: 0,
    };
    si.commit(h, &batch, p).unwrap();
}

fn note(h: &mut Host, s: &SimSession, key: &str, text: &str, tag: &[&str]) -> String {
    let ev = h.event(&format!("{}/{key}", s.sid), &s.sid, ActorKind::User, text);
    let mut input = observation(text, ev, &[], key);
    input.tags = tags(tag);
    s.record(h, input).unwrap().reference.unwrap()
}

/// One batch of `s`: recall and observe, then accept what `shows` keeps.
fn batch(
    h: &Host,
    s: &mut SimSession,
    text: &str,
    shows: impl Fn(&Change) -> bool,
) -> BatchDelivery {
    let entry = BatchEntry {
        recall: !text.is_empty(),
        text: text.to_string(),
    };
    s.obs.decay(h.memory.now_ms(), &s.cfg);
    let d = observe_batch(&h.memory, &s.caller, &s.obs, &entry, &s.budget, &s.cfg).unwrap();
    let displayed: BTreeSet<String> = d.items.iter().filter(|c| shows(c)).map(Change::key).collect();
    s.obs.commit_batch(&d, &displayed, &s.caller, h.memory.now_ms(), &s.cfg);
    d
}

fn refs(d: &BatchDelivery) -> Vec<String> {
    d.items.iter().map(|c| c.hint.reference.clone()).collect()
}

// ---------------------------------------------------------------------------
// the facade, batch by batch
// ---------------------------------------------------------------------------

/// §3.3 forgetting: every batch decays the tags first; only a tag the
/// conversation mentions again is reinforced; a recall hit or a restore is
/// no mention; time offline counts.
#[test]
fn tags_decay_every_batch_and_only_a_mention_reinforces_them() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    cognition(&mut h, &si, "item_weekly", "bob 的周报控制在一页以内", &["weekly-report"], &[BOB]);
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let cfg = b.cfg.clone();
    let t0 = h.memory.now_ms();
    b.obs.reply_topic("周报", &tags(&["weekly-report", "project-a"]), t0, &cfg).unwrap();
    // The weekly report is mentioned again ten minutes later; project A not.
    h.clock.advance(Duration::from_millis(10 * MIN));
    b.obs.reply_topic("", &tags(&["weekly-report"]), h.memory.now_ms(), &cfg).unwrap();
    assert_eq!(b.obs.topic.title, "周报", "an empty title keeps the current one");
    let touched = |b: &SimSession, t: &str| b.obs.topic.tags.iter().find(|x| x.tag == t).map(|x| (x.score, x.touched_at_ms));
    let reinforced = touched(&b, "weekly-report").unwrap();
    assert!(reinforced.0 > 1.5 && reinforced.1 == h.memory.now_ms());
    assert_eq!(touched(&b, "project-a").unwrap(), (1.0, t0));
    // A batch whose recall hits through the tag reinforces nothing.
    let d = batch(&h, &mut b, "", |_| true);
    assert_eq!(refs(&d), ["item_weekly@1"]);
    assert_eq!(touched(&b, "weekly-report").unwrap(), reinforced);
    // The session goes away for 90 minutes and comes back from its saved
    // state: decay counts the time offline, before anything is recalled.
    let saved = serde_json::to_string(&b.obs).unwrap();
    h.clock.advance(Duration::from_millis(90 * MIN));
    b.obs = serde_json::from_str(&saved).unwrap();
    assert_eq!(touched(&b, "weekly-report").unwrap(), reinforced, "restoring is no mention");
    let evicted = b.obs.decay(h.memory.now_ms(), &cfg);
    assert_eq!(evicted, ["project-a"]);
    assert_eq!(b.obs.tags(), ["weekly-report"]);
    // Long enough without a mention: nothing is left, the base scope stays.
    h.clock.advance(Duration::from_millis(60 * MIN));
    assert_eq!(b.obs.decay(h.memory.now_ms(), &cfg), ["weekly-report"]);
    assert!(b.obs.tags().is_empty());
    assert_eq!(b.obs.topic.base_scope.subjects, [BOB.to_string(), AGENT.to_string()]);
    // The memory itself stays.
    assert!(h.memory.graph_view().unwrap().item("item_weekly").is_some());
    // Capacity: the lowest score goes; a clear topic switch drops what the
    // new topic does not mention.
    let now = h.memory.now_ms();
    let many: Vec<String> = (0..8).map(|i| format!("t{i}")).collect();
    b.obs.reply_topic("话题一", &many, now, &cfg).unwrap();
    h.clock.advance(Duration::from_millis(MIN));
    let u = b.obs.reply_topic("话题一", &tags(&["t9"]), h.memory.now_ms(), &cfg).unwrap();
    assert_eq!(b.obs.topic.tags.len(), 8);
    assert_eq!(u.evicted.len(), 1);
    let u = b.obs.reply_topic("完全不同的另一件事", &tags(&["t9"]), h.memory.now_ms(), &cfg).unwrap();
    assert_eq!(b.obs.tags(), ["t9"], "{u:?}");
}

/// §3.2 semi-subscription: with its tags unchanged and without repeating
/// them, a session learns of a related note another session wrote, then of
/// the cognition it became (and the note's disposition); an event-only
/// batch (no text, no recall) is an observation too.
#[test]
fn a_held_tag_brings_new_material_without_repeating_its_words() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let a = h.session("A", "ui-a", &[BOB, AGENT], &[BOB], &[]);
    let cfg = b.cfg.clone();
    b.obs.reply_topic("周报安排", &tags(&["weekly-report"]), h.memory.now_ms(), &cfg).unwrap();
    // The first batch builds the observation (nothing held yet).
    let d = batch(&h, &mut b, "", |_| true);
    assert!(d.rebuilt && d.items.is_empty());
    assert!(b.obs.snapshot.is_some());
    // Another conversation notes something tagged the same way.
    let r = note(&mut h, &a, "a1", "bob 说以后周报控制在一页以内", &["weekly-report"]);
    h.clock.advance(Duration::from_millis(MIN));
    let d = batch(&h, &mut b, "", |_| true);
    assert_eq!(refs(&d), [r.clone()]);
    assert_eq!(d.items[0].hint.state, HintState::Pending);
    assert!(d.recall.is_none(), "an event-only batch only observes");
    let line = libopendan::runner::memory_batch::line(&d.items[0], &d.corrections);
    assert!(line.text.starts_with("noted, not yet consolidated: bob 说以后周报控制在一页以内 (from session ui-a;"), "{}", line.text);
    // Shown once: not again.
    assert!(batch(&h, &mut b, "", |_| true).items.is_empty());
    // It is consolidated into a cognition carrying the tag.
    si.run(&mut h, 10, |_, _| {
        let ev = SourceRef::event(SourceType::SessionEvent, "ui-a/a1");
        plan(
            "c1",
            "one page",
            vec![
                obs_op("obs_one_page", ObservationKind::ExplicitStatement, "周报控制在一页以内", &ev, scope(&[BOB], &[])),
                ItemSpec { tags: &["weekly-report"], ..ItemSpec::new("item_one_page", "preference", "bob 的周报控制在一页以内", &["obs_one_page"], scope(&[BOB], &[]), Basis::UserStatement) }.op(),
            ],
            vec![memory_sim::absorbed(&r, &["item_one_page"])],
        )
    })
    .unwrap();
    let d = batch(&h, &mut b, "", |_| true);
    let kinds: Vec<(String, ChangeKind)> = d.items.iter().map(|c| (c.hint.reference.clone(), c.kind.clone())).collect();
    assert!(kinds.contains(&("item_one_page@1".into(), ChangeKind::NewCognition)), "{kinds:?}");
    assert!(kinds.contains(&(r.clone(), ChangeKind::PerceptionDisposed)), "{kinds:?}");
    let marker = d.items.iter().find(|c| c.kind == ChangeKind::PerceptionDisposed).unwrap();
    let marker = libopendan::runner::memory_batch::line(marker, &d.corrections).text;
    assert_eq!(marker, format!("note {r} was consolidated into item_one_page (the same material, not new evidence)"));
    assert_eq!(b.obs.read_set.get("item_one_page"), Some(&1));
}

/// A forgotten tag no longer brings new material; a cognition the session
/// read still brings its revisions (read set), once per version.
#[test]
fn a_forgotten_tag_stops_new_matches_but_read_cognitions_still_revise() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    cognition(&mut h, &si, "item_weekly", "bob 的周报控制在一页以内", &["weekly-report"], &[BOB]);
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let a = h.session("A", "ui-a", &[BOB, AGENT], &[BOB], &[]);
    let cfg = b.cfg.clone();
    // The title shares no word with the later note: only the tag could
    // bring it (a title, another tag or the batch text still would).
    b.obs.reply_topic("篇幅要求", &tags(&["weekly-report"]), h.memory.now_ms(), &cfg).unwrap();
    assert_eq!(refs(&batch(&h, &mut b, "", |_| true)), ["item_weekly@1"]);
    h.clock.advance(Duration::from_millis(3 * 60 * MIN));
    note(&mut h, &a, "a2", "周报改成每两周一次", &["weekly-report"]);
    let d = batch(&h, &mut b, "", |_| true);
    assert!(b.obs.tags().is_empty(), "the tag decayed away first");
    assert!(d.items.is_empty(), "{:?}", refs(&d));
    cognition_rev(&mut h, &si, "item_weekly", "bob 的周报控制在一页以内，附件不限", &["weekly-report"], &[BOB], Some(1));
    let d = batch(&h, &mut b, "", |_| true);
    assert_eq!(refs(&d), ["item_weekly@2"]);
    assert_eq!(d.items[0].kind, ChangeKind::ReadRevision { from: 1 });
    let line = libopendan::runner::memory_batch::line(&d.items[0], &d.corrections).text;
    assert!(line.starts_with("hint revised (you saw @1): bob 的周报控制在一页以内，附件不限 ("), "{line}");
    assert!(batch(&h, &mut b, "", |_| true).items.is_empty(), "same version, same state: once");
}

/// §3.4: the recall and the observation of one batch share one budget;
/// duplicates are merged, validity changes go first, the shares borrow, and
/// what does not fit waits as pending (first next time).
#[test]
fn recall_and_observation_share_one_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    for i in 0..3 {
        cognition(&mut h, &si, &format!("item_read{i}"), &format!("项目 A 的周报规则 {i}"), &["weekly-report"], &[BOB]);
    }
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let a = h.session("A", "ui-a", &[BOB, AGENT], &[BOB], &[]);
    let cfg = b.cfg.clone();
    b.obs.reply_topic("项目 A 的周报", &tags(&["weekly-report"]), h.memory.now_ms(), &cfg).unwrap();
    assert_eq!(batch(&h, &mut b, "", |_| true).items.len(), 3);
    // Three read cognitions are revised, seven new ones and two notes match.
    for i in 0..3 {
        cognition_rev(&mut h, &si, &format!("item_read{i}"), &format!("项目 A 的周报规则 {i}（修订）"), &["weekly-report"], &[BOB], Some(1));
    }
    for i in 0..7 {
        cognition(&mut h, &si, &format!("item_new{i}"), &format!("项目 A 的周报还要注意第 {i} 点"), &["weekly-report"], &[BOB]);
    }
    note(&mut h, &a, "n1", "项目 A 的周报下周起改由 carol 汇总", &["weekly-report"]);
    note(&mut h, &a, "n2", "项目 A 的周报模板换新版", &["weekly-report"]);
    // The batch text finds the new cognitions too: one copy each.
    let d = batch(&h, &mut b, "项目 A 的周报这周怎么写？", |_| true);
    assert_eq!(d.items.len(), 8);
    let pools: Vec<usize> = d.items.iter().map(Change::pool).collect();
    assert_eq!(&pools[..3], [0, 0, 0], "validity changes first: {pools:?}");
    let keys: BTreeSet<String> = d.items.iter().map(Change::key).collect();
    assert_eq!(keys.len(), 8, "no duplicate");
    assert_eq!(pools.iter().filter(|p| **p == 2).count(), 2, "the notes' share: {pools:?}");
    assert_eq!(pools.iter().filter(|p| **p == 1).count(), 3);
    assert!(!d.overflow.is_empty());
    let waiting = b.obs.pending.clone();
    assert_eq!(waiting.len(), d.overflow.len());
    // Next batch: what waited comes first, nothing shown twice.
    let d2 = batch(&h, &mut b, "", |_| true);
    assert_eq!(d2.items.len(), waiting.len().min(8), "{:?}", refs(&d2));
    for c in &d2.items {
        assert!(!keys.contains(&c.key()), "{} again", c.key());
    }
    assert!(b.obs.pending.is_empty());
}

/// §3.3: only what the rendered input shows is accepted; the rest waits.
/// Too much waiting asks for a rebuild, which the next batch does without
/// a new topic.
#[test]
fn only_what_is_shown_is_accepted_and_too_much_waiting_rebuilds() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    for i in 0..3 {
        cognition(&mut h, &si, &format!("item_c{i}"), &format!("bob 喜欢的周报格式 {i}"), &["weekly-report"], &[BOB]);
    }
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let cfg = b.cfg.clone();
    b.obs.reply_topic("周报", &tags(&["weekly-report"]), h.memory.now_ms(), &cfg).unwrap();
    // A template that placed only the first clue.
    let d = batch(&h, &mut b, "", |c| c.hint.id == "item_c0");
    assert_eq!(d.items.len(), 3);
    assert_eq!(b.obs.shown.len(), 1);
    assert_eq!(b.obs.read_set.keys().collect::<Vec<_>>(), ["item_c0"]);
    assert_eq!(b.obs.pending, ["c:item_c1", "c:item_c2"]);
    let d = batch(&h, &mut b, "", |_| true);
    assert_eq!(refs(&d), ["item_c1@1", "item_c2@1"]);
    // Pending over its cap: a resync, done by the next batch by itself.
    let mut tight = cfg.clone();
    tight.pending_cap = 1;
    let d = BatchDelivery { overflow: vec!["c:item_c1".into(), "c:item_c2".into()], snapshot: b.obs.snapshot.clone(), ..BatchDelivery::default() };
    b.obs.commit_batch(&d, &BTreeSet::new(), &b.caller.clone(), h.memory.now_ms(), &tight);
    assert!(b.obs.resync_required && b.obs.pending.is_empty());
    let d = batch(&h, &mut b, "", |_| true);
    assert!(d.rebuilt && d.recall.is_some(), "rebuilt without a new topic");
    assert!(!b.obs.resync_required);
}

/// M-25: a group conversation whose words match what the owner said in
/// private sees nothing of it (cognitions and notes alike).
#[test]
fn a_group_never_sees_the_owners_private_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let mut h = host(&tmp);
    let si = h.si("si");
    cognition(&mut h, &si, "item_private", "bob 下周三去医院复查", &["hospital"], &[BOB]);
    let a = h.session("A", "ui-a", &[BOB, AGENT], &[BOB], &[]);
    note(&mut h, &a, "p1", "bob 说医院复查改到周四", &["hospital"]);
    let mut g = h.session("G", "ui-g", &[GROUP, AGENT], &[GROUP], &[]);
    let cfg = g.cfg.clone();
    g.obs.reply_topic("医院复查", &tags(&["hospital"]), h.memory.now_ms(), &cfg).unwrap();
    let d = batch(&h, &mut g, "bob 下周去医院复查是哪天？", |_| true);
    assert!(d.items.is_empty(), "{:?}", refs(&d));
    note(&mut h, &a, "p2", "bob 医院复查又改到周五", &["hospital"]);
    assert!(batch(&h, &mut g, "", |_| true).items.is_empty());
    // The same words in bob's own conversation do find them.
    let mut b = h.session("B", "ui-b", &[BOB, AGENT], &[BOB], &[]);
    let d = batch(&h, &mut b, "bob 下周去医院复查是哪天？", |_| true);
    assert!(refs(&d).contains(&"item_private@1".to_string()), "{:?}", refs(&d));
    // Carol's conversation does not.
    let mut c = h.session("C", "ui-c", &[CAROL, AGENT], &[CAROL], &[]);
    assert!(batch(&h, &mut c, "bob 下周去医院复查是哪天？", |_| true).items.is_empty());
}

// ---------------------------------------------------------------------------
// the runner
// ---------------------------------------------------------------------------

fn ui_spec(peer: &str) -> libopendan::api::SessionSpec {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3A{}", peer.replace(':', "%3A")));
    spec.owner = Some(BOB.into());
    spec.outbound = Some(OutboundBinding {
        to: peer.into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec.prompt.llm_context = json!({
        "json_schema": {
            "type": "object", "additionalProperties": false,
            "required": ["reply", "topic", "artifacts"],
            "properties": {
                "reply": { "type": "string" },
                "topic": { "type": ["object", "null"], "additionalProperties": false,
                           "required": ["title", "tags"],
                           "properties": { "title": { "type": "string" },
                                           "tags": { "type": "array", "items": { "type": "string" } } } },
                "artifacts": { "type": "array", "items": { "type": "string" } }
            }
        }
    });
    spec
}

fn hinted_deps(env: &Env, llm: std::sync::Arc<ScriptedLlm>) -> libopendan::runner::RunnerDeps {
    let mut deps = env.deps(llm);
    deps.options.load_hints = true;
    deps
}

fn seeded(env: &Env) -> Host {
    let mut h = Host::new(&env.agent_root, "2026-11-09T09:00:00+08:00");
    let si = h.si("seed");
    cognition(&mut h, &si, "item_weekly_format", "bob 要求项目 A 的周报先给结论，明细放附件", &["weekly-report", "project-a"], &[BOB]);
    cognition(&mut h, &si, "item_weekly_length", "bob 要求项目 A 的周报控制在一页以内", &["weekly-report"], &[BOB]);
    cognition(&mut h, &si, "item_weather", "bob 问天气没说地点时默认查上海", &["weather"], &[BOB]);
    drop(si);
    h
}

fn hint_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("- hint") || l.starts_with("- noted") || l.starts_with("- memory"))
        .map(str::to_string)
        .collect()
}

/// R1: a new conversation asked about something remembered has the clues
/// on its first turn (no `recall` needed), its state follows the batch
/// receipt, and the next turn does not repeat them.
#[tokio::test]
async fn a_new_conversation_has_related_hints_on_its_first_turn() {
    let env = Env::new();
    let _h = seeded(&env);
    let sd = env.create_work(ui_spec(BOB)).await;
    let llm = ScriptedLlm::new(|req, n| {
        let input = last_user_text(req);
        match n {
            0 => {
                let lines = hint_lines(&input);
                assert_eq!(lines.len(), 2, "{input}");
                assert!(input.contains("- hint: bob 要求项目 A 的周报先给结论，明细放附件 (the user's explicit statement") || input.contains("- hint: bob 要求项目 A 的周报先给结论，明细放附件 (stated by the user"), "{input}");
                assert!(input.contains("ref item_weekly_length@1)"), "{input}");
                assert!(!input.contains("天气"), "{input}");
                text(r#"{"reply":"先给结论，明细放附件，一页以内。","topic":{"title":"项目 A 的周报","tags":["weekly-report"]},"artifacts":[]}"#)
            }
            1 => {
                assert!(hint_lines(&input).is_empty(), "shown once: {input}");
                text(r#"{"reply":"好的。","topic":null,"artifacts":[]}"#)
            }
            _ => panic!("unexpected inference {n}"),
        }
    });
    let deps = hinted_deps(&env, llm.clone());
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("项目 A 的周报有什么要求？"));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{r:?}");
    let state = sd.state().unwrap();
    let obs = state.memory.clone().expect("observation saved");
    assert_eq!(obs.read_set.keys().collect::<Vec<_>>(), ["item_weekly_format", "item_weekly_length"]);
    assert!(obs.snapshot.is_some());
    // The reply's topic: kept with its score, and seen by other sessions.
    assert_eq!(obs.tags(), ["weekly-report"]);
    assert_eq!(state.topic.title, "项目 A 的周报");
    assert_eq!(state.topic.tags, ["weekly-report"]);
    // The observation is the one the batch receipt carried.
    let run = state.last_run.clone().unwrap();
    let (_, snap) = sd.runs().load_checked(&run).unwrap();
    let host = snap.unwrap().state.host.unwrap();
    let meta: HostMeta = serde_json::from_value(host[HOST_META_KEY].clone()).unwrap();
    let carried: ObservationState = serde_json::from_value(meta.input_receipts[0].extra[RECEIPT_MEMORY].clone()).unwrap();
    assert_eq!(carried.read_set, obs.read_set);
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("好的，谢谢"));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{r:?}");
    assert_eq!(llm.count(), 2);
}

/// M-25 through the runner: a group conversation asking about the owner's
/// private matters gets no clue of them.
#[tokio::test]
async fn a_group_conversation_gets_no_clue_of_the_owners_private_memory() {
    let env = Env::new();
    let _h = seeded(&env);
    let sd = env.create_work(ui_spec(GROUP)).await;
    let llm = ScriptedLlm::new(|req, n| {
        let input = last_user_text(req);
        assert!(hint_lines(&input).is_empty(), "{input}");
        assert_eq!(n, 0);
        text(r#"{"reply":"这个我不方便在群里说。","topic":null,"artifacts":[]}"#)
    });
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("项目 A 的周报有什么要求？"));
    let r = drive(&sd, &hinted_deps(&env, llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    assert_eq!(llm.count(), 1);
}

/// R2: a Work starts with what is known about its task (at most 4 clues,
/// none unrelated), keeps its title and objective as its topic, and later
/// batches observe new related material.
#[tokio::test]
async fn a_work_starts_with_what_is_known_about_its_task() {
    let env = Env::new();
    let mut h = seeded(&env);
    let si = h.si("seed2");
    for i in 0..4 {
        cognition(&mut h, &si, &format!("item_weekly_more{i}"), &format!("项目 A 的周报第 {i} 条约定"), &["weekly-report"], &[BOB]);
    }
    drop(si);
    let mut spec = SessionTemplate::load("work", None).unwrap().spec("整理项目 A 本周的周报");
    spec.title = Some("项目 A 周报".into());
    spec.owner = Some(BOB.into());
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    assert!(spec.policy.load_hints, "a Work recalls by default (D4)");
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        let input = last_user_text(req);
        match n {
            0 => {
                let lines = hint_lines(&input);
                assert!(!lines.is_empty() && lines.len() <= 4, "{input}");
                assert!(lines.iter().all(|l| l.contains("周报")), "{lines:?}");
                assert!(!input.contains("天气"), "{input}");
                text("started")
            }
            _ => text("still working"),
        }
    });
    let deps = hinted_deps(&env, llm.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    let obs = sd.state().unwrap().memory.expect("observation saved");
    assert_eq!(obs.topic.title, "项目 A 周报\n整理项目 A 本周的周报");
    assert!(obs.tags().is_empty());
    assert!(obs.topic.base_scope.subjects.contains(&AGENT.to_string()));
    assert_eq!(obs.read_set.len() + obs.pending.len(), 6, "{obs:?}");
}

/// TD-01: a store that cannot be read is said (one line, a worklog record),
/// never an empty recall, and no observation progress moves.
#[tokio::test]
async fn an_unreadable_memory_is_said_not_hidden() {
    let env = Env::new();
    let _h = seeded(&env);
    let log = env.agent_root.join("memory/.meta/occasions.jsonl");
    let log_text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, log_text.replacen("一页以内", "两页以内", 1)).unwrap();
    let sd = env.create_work(ui_spec(BOB)).await;
    let llm = ScriptedLlm::new(|req, _| {
        let input = last_user_text(req);
        assert!(input.contains("- memory unavailable: "), "{input}");
        assert!(hint_lines(&input).iter().all(|l| l.starts_with("- memory unavailable")), "{input}");
        text(r#"{"reply":"记忆暂时读不到。","topic":null,"artifacts":[]}"#)
    });
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("项目 A 的周报有什么要求？"));
    let r = drive(&sd, &hinted_deps(&env, llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    let obs = sd.state().unwrap().memory.expect("the observation is still saved");
    assert!(obs.snapshot.is_none() && obs.read_set.is_empty() && obs.shown.is_empty());
    let worklog = read_worklog(&sd);
    assert!(worklog.iter().any(|e| matches!(&e.body, WorklogBody::TurnStarted { memory_error: Some(m), .. } if m.contains("corrupt"))), "{worklog:?}");
}

/// R3: the active query answers as the session that asks — its grants, not
/// arguments — through the Agent State and through `agent-session recall`.
#[tokio::test]
async fn recall_answers_as_the_session_that_asks() {
    use libopendan::state::AgentStateClient;
    let env = Env::new();
    let mut h = seeded(&env);
    let mine = env.create_work(ui_spec(BOB)).await;
    let group = env.create_work(ui_spec(GROUP)).await;
    // A note of bob's conversation, not yet consolidated.
    let a = h.session("A", mine.sid(), &[BOB, AGENT], &[BOB], &[]);
    let n = note(&mut h, &a, "n1", "bob 说项目 A 的周报以后周五前发", &["weekly-report"]);
    let q = MemoryQuery {
        text: Some("项目 A 的周报".into()),
        tags: tags(&["weekly-report"]),
        include_recent_perceptions: true,
        ..MemoryQuery::default()
    };
    let agent = env.agent();
    let r = agent.cognition().query(mine.sid(), &q).await.unwrap();
    let ids: Vec<&str> = r.cognitions.iter().map(|h| h.id.as_str()).collect();
    assert!(ids.contains(&"item_weekly_format") && ids.contains(&"item_weekly_length"), "{ids:?}");
    assert_eq!(r.recent_perceptions.iter().map(|h| h.reference.clone()).collect::<Vec<_>>(), [n.clone()]);
    let g = agent.cognition().query(group.sid(), &q).await.unwrap();
    assert!(g.cognitions.is_empty() && g.recent_perceptions.is_empty(), "{g:?}");
    match agent.cognition().read(mine.sid(), "item_weekly_length@1").await.unwrap() {
        MemoryRead::Cognition { cognition, provenance } => {
            assert_eq!(cognition.item.item_id, "item_weekly_length");
            assert_eq!(provenance.current_basis.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(agent.cognition().read(group.sid(), "item_weekly_length").await.is_err());
    // The shell command: who asks comes from the environment the host set.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["recall", "项目 A 的周报", "weekly-report"])
        .env("OPENDAN_SESSION_ID", mine.sid())
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("cognitions (2):\n- hint: "), "{stdout}");
    assert!(stdout.contains(&format!("noted, not yet consolidated: bob 说项目 A 的周报以后周五前发 (from session {};", mine.sid())), "{stdout}");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["recall", "--read", "item_weekly_length"])
        .env("OPENDAN_SESSION_ID", mine.sid())
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && stdout.contains("evidence:\n- bob 要求项目 A 的周报控制在一页以内"), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["recall", "项目 A 的周报"])
        .env_remove("OPENDAN_SESSION_ID")
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .output()
        .unwrap();
    assert!(!out.status.success(), "no session, no answer");
}
