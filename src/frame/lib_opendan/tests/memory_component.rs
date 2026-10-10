//! Memory component tests beyond the narrative scenarios: concurrency,
//! failure and recovery, budgets, topic lifecycle, two processes (TODO §6,
//! T04).

#[path = "../examples/support/memory_sim.rs"]
mod memory_sim;

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_tool::agent_memory::{ObservationKind, ReinforceObjectWeightOp};
use fs2::FileExt;
use libopendan::lock::{Acquire, Lease};
use libopendan::memory::*;
use libopendan::protocol::HolderInfo;
use libopendan::state::perception::read_records;
use libopendan::state::{AgentStateClient, FsAgentStateClient, RecallQuery as RunnerRecall};
use memory_sim::*;
use tempfile::TempDir;

fn host() -> (TempDir, Host) {
    let tmp = TempDir::new().unwrap();
    let h = Host::new(&tmp.path().join("agent"), "2026-10-09T09:00:00+08:00");
    (tmp, h)
}

fn holder(name: &str) -> HolderInfo {
    HolderInfo {
        runner_id: name.into(),
        principal: name.into(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    }
}

fn gseq(h: &Host) -> u64 {
    h.memory.graph().unwrap().graph_seq().unwrap()
}

fn plain_memory(root: &std::path::Path) -> Memory {
    Memory::open(root, MemoryConfig::default())
}

fn session_lease(root: &std::path::Path, sid: &str) -> Lease {
    let path = root.join("sim-sessions").join(sid).join("lease.json");
    match Lease::acquire(&format!("session:{sid}"), &path, holder(sid)).unwrap() {
        Acquire::Acquired(l) => l,
        Acquire::Busy(_) => panic!("busy"),
    }
}

fn input(text: &str, ev: &str, objects: &[&str], key: &str) -> PerceptionInput {
    let mut s = SourceRef::event(SourceType::SessionEvent, ev);
    s.actor_kind = Some(ActorKind::User);
    observation(text, s, objects, key)
}

/// p written by A, seeded c1 consolidated from it.
fn seed(h: &mut Host, si: &SimSi) {
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "游戏页静态、不闪烁。");
    a.record(
        h,
        observation(
            "u1 要求 alpha 游戏页静态、不闪烁背景",
            e.clone(),
            &[ALPHA_GAME],
            "A/e1/1",
        ),
    )
    .unwrap();
    si.run(h, 10, |_, _| {
        plan(
            "seed",
            "seed",
            vec![
                obs_op(
                    "obs_seed",
                    ObservationKind::ExplicitStatement,
                    "游戏页静态、不闪烁",
                    &e,
                    scope(&[U1], &[ALPHA_GAME]),
                ),
                ItemSpec {
                    explicit: true,
                    tags: &["背景效果"],
                    ..ItemSpec::new(
                        "item_c1",
                        "preference",
                        "u1 希望 alpha 游戏页背景静态、不闪烁",
                        &["obs_seed"],
                        scope(&[U1], &[ALPHA_GAME]),
                        Basis::UserStatement,
                    )
                }
                .op(),
            ],
            vec![absorbed("ui-a:1", &["item_c1"])],
        )
    })
    .unwrap();
}

#[test]
fn s03_concurrent_writes_never_fall_into_the_gap() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.budget = Budget::of(100);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let root = h.root.clone();
    let writer = std::thread::spawn(move || {
        let m = plain_memory(&root);
        let lease = session_lease(&root, "ui-w");
        let caller = Caller::new("ui-w", &[U1, A1], &[U1]);
        for i in 0..40 {
            m.record_perception(
                &caller,
                &lease,
                input(
                    &format!("背景观察 {i}"),
                    &format!("W/e{i}"),
                    &[ALPHA_GAME],
                    &format!("W/e{i}/1"),
                ),
            )
            .unwrap();
            if i % 7 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    });
    let mut seen: Vec<String> = Vec::new();
    let mut rounds = 0;
    while !writer.is_finished() || rounds < 2 {
        let d = b.observe(&mut h).unwrap();
        seen.extend(d.changes.iter().map(|c| c.hint.id.clone()));
        if rounds == 3 {
            // A recall in the middle: overlap with later changes is deduped.
            let r = h
                .memory
                .query_topic(
                    &b.caller,
                    &TopicQuery::new(b.obs.query_scope(), Budget::of(100)),
                )
                .unwrap();
            for x in r
                .perceptions
                .iter()
                .filter(|x| !b.obs.shown.contains_key(&x.reference))
            {
                seen.push(x.id.clone());
            }
            b.obs.accept_recall(&r, &b.caller, h.memory.now_ms());
        }
        if writer.is_finished() {
            rounds += 1;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    writer.join().unwrap();
    let d = b.observe(&mut h).unwrap();
    seen.extend(d.changes.iter().map(|c| c.hint.id.clone()));
    let unique: BTreeSet<&String> = seen.iter().collect();
    let expected: BTreeSet<String> = (1..=40).map(|i| format!("ui-w:{i}")).collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "a write was delivered twice: {seen:?}"
    );
    assert_eq!(
        unique.into_iter().cloned().collect::<BTreeSet<_>>(),
        expected
    );
}

#[test]
fn s04_s34_topic_lifecycle_and_valve() {
    let cfg = TopicConfig::default();
    let t0 = 1_000_000u64;
    let mut o = ObservationState::new(QueryScope {
        subjects: vec![U1.into()],
        objects: vec![ALPHA_GAME.into()],
    });
    let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let u = o
        .set_topic("调整游戏背景", &tags(&["背景", "动效"]), t0, &cfg)
        .unwrap();
    assert!(u.changed && u.revision == 1 && u.valve == Valve::Deep);
    o.read_set.insert("item_x".into(), 1);
    o.last_query = Some(LastQuery {
        at_ms: t0,
        topic_revision: 1,
        rounds_since: 0,
        key: String::new(),
        snapshot: MemorySnapshot::default(),
        had_corrections: false,
        expires_at: None,
    });
    let same = o
        .set_topic("调整游戏背景", &tags(&["背景", "动效"]), t0 + 1000, &cfg)
        .unwrap();
    assert!(
        !same.changed && same.revision == 1 && same.valve == Valve::None,
        "idempotent: {same:?}"
    );
    let light = o
        .set_topic("调整游戏背景", &tags(&["背景"]), t0 + 31 * 60 * 1000, &cfg)
        .unwrap();
    assert_eq!(light.valve, Valve::Light);
    o.last_query.as_mut().unwrap().rounds_since = 5;
    assert_eq!(
        o.set_topic("调整游戏背景", &[], t0 + 31 * 60 * 1000 + 1, &cfg)
            .unwrap()
            .valve,
        Valve::Light
    );
    o.last_query.as_mut().unwrap().rounds_since = 0;
    o.last_query.as_mut().unwrap().at_ms = t0 + 31 * 60 * 1000;
    let deep = o
        .set_topic(
            "调整游戏背景",
            &tags(&["配色", "图案"]),
            t0 + 32 * 60 * 1000,
            &cfg,
        )
        .unwrap();
    assert_eq!(deep.valve, Valve::Deep, "{deep:?}");
    // Capacity: never more than 8 tags; the most mentioned survives.
    let now = t0 + 33 * 60 * 1000;
    for i in 0..12 {
        o.set_topic(
            "调整游戏背景",
            &tags(&["背景", &format!("细节{i}")]),
            now + i * 1000,
            &cfg,
        )
        .unwrap();
    }
    assert!(o.topic.tags.len() <= cfg.capacity);
    assert!(o.tags().contains(&"背景".to_string()));
    // Long idle: decayed tags drop, the base scope stays.
    let later = now + 4 * 3600 * 1000;
    o.set_topic("调整游戏背景", &[], later, &cfg).unwrap();
    assert!(o.topic.tags.is_empty(), "{:?}", o.topic.tags);
    assert_eq!(o.topic.base_scope.objects, vec![ALPHA_GAME.to_string()]);
    // Switch and clear keep the read set.
    let before = o.topic.revision;
    let sw = o
        .set_topic("beta 计分数据", &tags(&["计分"]), later + 1, &cfg)
        .unwrap();
    assert!(sw.changed && o.topic.revision == before + 1 && o.tags() == vec!["计分".to_string()]);
    o.clear_topic();
    assert!(o.topic.title.is_empty() && o.tags().is_empty() && o.topic.revision == before + 2);
    assert_eq!(o.read_set.get("item_x"), Some(&1));
    // Persistence round trip.
    let tmp = TempDir::new().unwrap();
    o.save(&tmp.path().join("obs.json")).unwrap();
    assert_eq!(
        ObservationState::load(&tmp.path().join("obs.json"))
            .unwrap()
            .unwrap(),
        o
    );
}

#[test]
fn s05_idempotency_keys() {
    let (_t, mut h) = host();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "两条要求");
    let first = a
        .record(
            &mut h,
            observation("要求一", e.clone(), &[ALPHA_GAME], "A/e1/1"),
        )
        .unwrap();
    let again = a
        .record(
            &mut h,
            observation("要求一", e.clone(), &[ALPHA_GAME], "A/e1/1"),
        )
        .unwrap();
    assert_eq!(first.status, ReceiptStatus::Recorded);
    assert_eq!(
        (again.status, again.reference.clone()),
        (ReceiptStatus::Replayed, first.reference.clone())
    );
    assert!(matches!(
        a.record(
            &mut h,
            observation("要求一（改）", e.clone(), &[ALPHA_GAME], "A/e1/1")
        ),
        Err(MemoryError::Conflict(_))
    ));
    let second = a
        .record(
            &mut h,
            observation("要求二", e.clone(), &[ALPHA_GAME], "A/e1/2"),
        )
        .unwrap();
    assert_eq!(
        second.reference.as_deref(),
        Some("ui-a:2"),
        "two observations of one event are both kept"
    );
    let si = h.si("si");
    si.run(&mut h, 10, |_, _| {
        plan(
            "d",
            "discard",
            Vec::new(),
            vec![discarded("ui-a:1", "test"), discarded("ui-a:2", "test")],
        )
    })
    .unwrap();
    let replay = a
        .record(
            &mut h,
            observation("要求一", e.clone(), &[ALPHA_GAME], "A/e1/1"),
        )
        .unwrap();
    assert!(
        replay.status == ReceiptStatus::Replayed && replay.cleared,
        "{replay:?}"
    );
    assert!(matches!(
        a.record(
            &mut h,
            observation("要求一（改）", e, &[ALPHA_GAME], "A/e1/1")
        ),
        Err(MemoryError::Conflict(_))
    ));
    let recs = read_records(&h.layout().perception_file("ui-a")).unwrap();
    assert_eq!(recs.len(), 2);
    assert!(recs
        .iter()
        .all(|r| r.cleared.is_some() && r.summary.is_empty()));
}

#[test]
fn s07_s09_batches_are_fixed_and_terminal_dispositions_conflict() {
    let (_t, mut h) = host();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    for i in 1..=2 {
        let e = h.event(&format!("A/e{i}"), "ui-a", ActorKind::User, "x");
        a.record(
            &mut h,
            observation(&format!("观察 {i}"), e, &[ALPHA_GAME], &format!("A/e{i}/1")),
        )
        .unwrap();
    }
    let si = h.si("si");
    let b1 = si.begin(&mut h, 10).unwrap();
    let b2 = si.begin(&mut h, 10).unwrap();
    let e3 = h.event("A/e3", "ui-a", ActorKind::User, "x");
    a.record(
        &mut h,
        observation("处理中新增", e3, &[ALPHA_GAME], "A/e3/1"),
    )
    .unwrap();
    si.commit(
        &mut h,
        &b1,
        plan("b1", "b1", Vec::new(), vec![discarded("ui-a:1", "noise")]),
    )
    .unwrap();
    assert!(matches!(
        si.commit(
            &mut h,
            &b2,
            plan(
                "b2",
                "b2",
                Vec::new(),
                vec![discarded("ui-a:1", "noise again")]
            )
        ),
        Err(MemoryError::Conflict(_))
    ));
    assert!(matches!(
        si.commit(
            &mut h,
            &b1,
            plan(
                "b3",
                "b3",
                Vec::new(),
                vec![discarded("ui-a:3", "not in batch")]
            )
        ),
        Err(MemoryError::Invalid(_))
    ));
    si.commit(
        &mut h,
        &b2,
        plan("b4", "b4", Vec::new(), vec![discarded("ui-a:2", "noise")]),
    )
    .unwrap();
    let pw = h.memory.pending_work().unwrap();
    assert_eq!((pw.new, pw.retry_cleanup), (1, 2), "{pw:?}");
}

#[test]
fn s08_commit_faults_through_the_facade() {
    let (_t, mut h) = host();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "x");
    a.record(&mut h, observation("观察", e, &[ALPHA_GAME], "A/e1/1"))
        .unwrap();
    let si = h.si("si");
    let b = si.begin(&mut h, 10).unwrap();
    let p = plan("f", "fault", Vec::new(), vec![discarded("ui-a:1", "noise")]);
    h.graph_fault.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        si.commit(&mut h, &b, p.clone()),
        Err(MemoryError::Unavailable(_))
    ));
    assert_eq!(gseq(&h), 0);
    assert_eq!(
        h.memory.pending_work().unwrap().new,
        1,
        "nothing lost before the append"
    );
    h.graph_fault.store(2, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        si.commit(&mut h, &b, p.clone()),
        Err(MemoryError::CommitUnknown(_))
    ));
    assert_eq!(
        h.memory.graph_view().unwrap().seq(),
        0,
        "readers ignore the torn tail"
    );
    let r = si.commit(&mut h, &b, p.clone()).unwrap();
    assert!(r.recovered_torn_tail && r.status == CommitStatus::Committed);
    let b = si.begin(&mut h, 10).unwrap();
    let e2 = h.event("A/e2", "ui-a", ActorKind::User, "y");
    a.record(&mut h, observation("观察2", e2, &[ALPHA_GAME], "A/e2/1"))
        .unwrap();
    let b2 = si.begin(&mut h, 10).unwrap();
    let p2 = plan(
        "f2",
        "fsync",
        Vec::new(),
        vec![discarded("ui-a:2", "noise")],
    );
    h.graph_fault.store(3, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        si.commit(&mut h, &b2, p2.clone()),
        Err(MemoryError::CommitUnknown(_))
    ));
    assert_eq!(
        si.commit(&mut h, &b2, p2).unwrap().status,
        CommitStatus::AlreadyCommitted
    );
    assert_eq!(gseq(&h), 2);
    let _ = b;
    let e3 = h.event("A/e3", "ui-a", ActorKind::User, "z");
    a.record(&mut h, observation("观察3", e3, &[ALPHA_GAME], "A/e3/1"))
        .unwrap();
    let b3 = si.begin(&mut h, 10).unwrap();
    h.graph_fault.store(4, std::sync::atomic::Ordering::SeqCst);
    let r = si
        .commit(
            &mut h,
            &b3,
            plan(
                "f3",
                "materialize",
                Vec::new(),
                vec![discarded("ui-a:3", "noise")],
            ),
        )
        .unwrap();
    assert!(matches!(r.derived, DerivedStatus::PendingRepair(_)));
    let mut b_sess = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    assert!(
        b_sess.topic(&mut h, "调整游戏背景效果", &[]).is_ok(),
        "reads do not depend on derived state"
    );
    let admin = h.memory.admin().unwrap();
    let rep = admin.verify(false).unwrap();
    assert!(
        !rep.has_unrecoverable() && !rep.derived_issues.is_empty(),
        "{rep:?}"
    );
    assert!(admin.repair_derived().unwrap());
    assert!(admin.verify(false).unwrap().is_clean());
}

#[test]
fn s11_pending_overflow_asks_for_a_resync() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.cfg.pending_cap = 2;
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let w = h.session("W", "ui-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    for i in 0..5 {
        let e = h.event(&format!("W/e{i}"), "ui-w", ActorKind::User, "x");
        w.record(
            &mut h,
            observation(
                &format!("背景观察 {i}"),
                e,
                &[ALPHA_GAME],
                &format!("W/e{i}/1"),
            ),
        )
        .unwrap();
    }
    b.observe_with(&mut h, &Budget::zero(), true).unwrap();
    assert!(b.obs.resync_required && b.obs.pending.is_empty());
    assert!(b.observe(&mut h).unwrap().resync_required);
    let out = b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    assert_eq!(out.update.valve, Valve::Deep);
    assert_eq!(out.result.unwrap().perceptions.len(), 5);
    assert!(!b.obs.resync_required);
    assert!(b.observe(&mut h).unwrap().changes.is_empty());
}

#[test]
fn s13_failures_never_look_like_empty_memory() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    // Writer lock held elsewhere: reads go on; a commit times out.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(h.root.join("memory/.meta/lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let quick = Memory::open(
        &h.root,
        MemoryConfig {
            lock_timeout: Duration::from_millis(100),
            ..MemoryConfig::default()
        },
    );
    let c = Caller::new("ui-b", &[U1, A1], &[U1]);
    assert!(quick
        .query_topic(&c, &TopicQuery::new(b.obs.query_scope(), Budget::default()))
        .is_ok());
    let si_quick = quick.consolidator(&si.lease, "si").unwrap();
    let bt = si_quick.begin(&BatchOptions { limit: 5 }).unwrap();
    assert!(matches!(
        si_quick.commit(&bt, plan("x", "x", Vec::new(), Vec::new())),
        Err(MemoryError::LockTimeout(_))
    ));
    FileExt::unlock(&lock).unwrap();
    // Not triggered vs looked-and-found-nothing.
    let none = h.memory.query(&c, &MemoryQuery::default()).unwrap();
    assert_eq!(none.status, RecallStatus::NotTriggered);
    let empty = h
        .memory
        .query(
            &c,
            &MemoryQuery {
                text: Some("完全无关的话题".into()),
                objects: vec![BETA_GAME.into()],
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    assert!(empty.status == RecallStatus::Recalled && empty.cognitions.is_empty());
    // Corruption: an error, and nothing advances.
    let log = h.root.join("memory/.meta/occasions.jsonl");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text.replacen("不闪烁", "会闪烁", 1)).unwrap();
    let w = h.session("W", "ui-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("W/e1", "ui-w", ActorKind::User, "x");
    w.record(&mut h, observation("背景观察", e, &[ALPHA_GAME], "W/e1/1"))
        .unwrap();
    h.restart();
    let before = b.obs.clone();
    assert!(matches!(
        observe(&h.memory, &b.caller, &b.obs, &b.budget, &b.cfg),
        Err(MemoryError::Corrupted(_))
    ));
    assert!(matches!(
        h.memory.query_topic(
            &b.caller,
            &TopicQuery::new(b.obs.query_scope(), Budget::default())
        ),
        Err(MemoryError::Corrupted(_))
    ));
    assert_eq!(b.obs, before);
    std::fs::write(&log, text).unwrap();
}

#[tokio::test]
async fn s13_runner_recall_reports_failures() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let client = FsAgentStateClient::open(&h.root, "did:bns:a1", None, None).unwrap();
    let ok = client
        .cognition()
        .recall_hints(&RunnerRecall {
            tags: vec!["背景效果".into()],
            max_hints: 5,
        })
        .await
        .unwrap();
    assert_eq!(ok.len(), 1);
    let none = client
        .cognition()
        .recall_hints(&RunnerRecall::default())
        .await
        .unwrap();
    assert!(
        none.is_empty(),
        "no entry: not triggered, not the whole Graph"
    );
    let log = h.root.join("memory/.meta/occasions.jsonl");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text.replacen("不闪烁", "会闪烁", 1)).unwrap();
    assert!(client
        .cognition()
        .recall_hints(&RunnerRecall {
            tags: vec!["背景效果".into()],
            max_hints: 5,
        })
        .await
        .is_err());
}

#[test]
fn s36_entries_and_tag_rules() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let c = Caller::new("ui-b", &[U1, A1], &[U1]);
    let q = |tags: &[&str]| MemoryQuery {
        tags: tags.iter().map(|s| s.to_string()).collect(),
        objects: vec![ALPHA_GAME.into()],
        ..MemoryQuery::default()
    };
    assert!(matches!(
        h.memory.query(&c, &q(&["a\"b", "x:y"])),
        Err(MemoryError::Invalid(_))
    ));
    assert_eq!(
        h.memory
            .query(&c, &q(&["a\"b", "背景效果"]))
            .unwrap()
            .cognitions
            .len(),
        1
    );
    assert!(
        matches!(
            h.memory.query(&c, &q(&["一二三四五六七八九十一"])),
            Err(MemoryError::Invalid(_))
        ),
        "33 bytes"
    );
    assert!(
        h.memory.query(&c, &q(&["背景 效果"])).is_ok(),
        "phrase tags are allowed"
    );
}

#[test]
fn s37_snapshots_unchanged_answers_and_pages() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let s0 = h.memory.snapshot().unwrap();
    h.advance_hours(1);
    assert_eq!(
        h.memory.snapshot().unwrap(),
        s0,
        "time alone does not move the snapshot"
    );
    let out = b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    assert_eq!(out.update.valve, Valve::Light);
    assert_eq!(out.result.unwrap().status, RecallStatus::Unchanged);
    // A perception elsewhere moves the snapshot: no longer "unchanged".
    let c = h.session("C", "ui-c", &[U1, A1], &[U1], &[BETA_GAME]);
    let e = h.event("C/e1", "ui-c", ActorKind::User, "x");
    c.record(&mut h, observation("beta 观察", e, &[BETA_GAME], "C/e1/1"))
        .unwrap();
    assert_ne!(h.memory.snapshot().unwrap(), s0);
    h.advance_hours(1);
    assert_eq!(
        b.topic(&mut h, "调整游戏背景效果", &[])
            .unwrap()
            .result
            .unwrap()
            .status,
        RecallStatus::Recalled
    );
    // A fork / compaction voids the cache.
    h.advance_hours(1);
    assert_eq!(
        b.topic(&mut h, "调整游戏背景效果", &[])
            .unwrap()
            .result
            .unwrap()
            .status,
        RecallStatus::Unchanged
    );
    b.obs.bump_context_generation();
    h.advance_hours(1);
    let r = b
        .topic(&mut h, "调整游戏背景效果", &[])
        .unwrap()
        .result
        .unwrap();
    assert!(
        r.status == RecallStatus::Recalled && !r.cognitions.is_empty(),
        "rebuilt context gets the cognition again"
    );
    // Expiry checked even when nothing was written.
    let e2 = h.event("A/e9", "ui-a", ActorKind::User, "这周用节日背景。");
    let tmp_until = iso(h.memory.now() + chrono::Duration::hours(3));
    si.run(&mut h, 10, |_, _| {
        plan(
            "temp",
            "temporary rule",
            vec![
                obs_op(
                    "obs_tmp",
                    ObservationKind::ExplicitStatement,
                    "这周用节日背景",
                    &e2,
                    scope(&[U1], &[ALPHA_GAME]),
                ),
                ItemSpec {
                    valid_until: Some(tmp_until.clone()),
                    tags: &["背景效果"],
                    ..ItemSpec::new(
                        "item_tmp",
                        "boundary",
                        "本周游戏页背景使用节日主题",
                        &["obs_tmp"],
                        scope(&[U1], &[ALPHA_GAME]),
                        Basis::UserStatement,
                    )
                }
                .op(),
            ],
            Vec::new(),
        )
    })
    .unwrap();
    h.advance_hours(1);
    let r = b
        .topic(&mut h, "调整游戏背景效果", &[])
        .unwrap()
        .result
        .unwrap();
    assert!(r.cognitions.iter().any(|x| x.id == "item_tmp"));
    h.advance_hours(3);
    let r = b
        .topic(&mut h, "调整游戏背景效果", &[])
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        r.status,
        RecallStatus::Recalled,
        "an expired item voids “unchanged”"
    );
    let full = h
        .memory
        .query(
            &b.caller,
            &MemoryQuery {
                text: Some("背景".into()),
                objects: vec![ALPHA_GAME.into()],
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    assert!(
        !full.cognitions.iter().any(|x| x.id == "item_tmp"),
        "expired cognitions are filtered"
    );
    // Pages share one snapshot.
    let page1 = h
        .memory
        .query(
            &b.caller,
            &MemoryQuery {
                text: Some("背景".into()),
                objects: vec![ALPHA_GAME.into()],
                limit: 1,
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    let cursor = page1.next_cursor.clone();
    assert!(cursor.is_none() || page1.cognitions.len() == 1);
    let e3 = h.event("C/e2", "ui-c", ActorKind::User, "y");
    c.record(
        &mut h,
        observation("另一条 beta 观察", e3, &[BETA_GAME], "C/e2/1"),
    )
    .unwrap();
    if let Some(cur) = cursor {
        let stale = h.memory.query(
            &b.caller,
            &MemoryQuery {
                text: Some("背景".into()),
                objects: vec![ALPHA_GAME.into()],
                limit: 1,
                cursor: Some(cur),
                ..MemoryQuery::default()
            },
        );
        assert!(matches!(stale, Err(MemoryError::StaleCursor(_))));
    }
}

#[test]
fn s27_only_the_consolidation_lease_writes_cognitions() {
    let (_t, mut h) = host();
    let b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    assert!(matches!(
        h.memory.consolidator(&b.lease, "ui-b"),
        Err(MemoryError::PermissionDenied(_))
    ));
    let other = h.session("O", "ui-o", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("B/e1", "ui-b", ActorKind::User, "x");
    assert!(matches!(
        h.memory.record_perception(
            &b.caller,
            &other.lease,
            observation("x", e, &[ALPHA_GAME], "B/e1/1")
        ),
        Err(MemoryError::PermissionDenied(_))
    ));
    let si = h.si("si");
    let batch = si.begin(&mut h, 5).unwrap();
    // The lease file is replaced under the holder: it lost the lease.
    let path = h.layout().lock_path(CONSOLIDATION_LEASE).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"{}").unwrap();
    assert!(matches!(
        si.commit(&mut h, &batch, plan("lost", "lost", Vec::new(), Vec::new())),
        Err(MemoryError::PermissionDenied(_))
    ));
    // Subjects outside the grants are refused.
    let e2 = h.event("B/e2", "ui-b", ActorKind::User, "x");
    let mut inp = observation("x", e2, &[ALPHA_GAME], "B/e2/1");
    inp.subjects = vec![U2.into()];
    assert!(matches!(
        h.memory.record_perception(&b.caller, &b.lease, inp),
        Err(MemoryError::PermissionDenied(_))
    ));
}

#[test]
fn s32_budget_pools_and_fade() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let e = h.event("A/e7", "ui-a", ActorKind::Agent, "推测用户偏好亮色");
    si.run(&mut h, 10, |_, _| {
        plan(
            "weak",
            "a weak but heavy inference",
            vec![
                obs_op(
                    "obs_weak",
                    ObservationKind::BehaviorSignal,
                    "推测偏好亮色",
                    &e,
                    scope(&[U1], &[ALPHA_GAME]),
                ),
                ItemSpec {
                    weight: 1.0,
                    confidence: 0.2,
                    tags: &["背景效果"],
                    ..ItemSpec::new(
                        "item_weak",
                        "tentative_understanding",
                        "u1 可能喜欢亮色背景（推断，未确认）",
                        &["obs_weak"],
                        scope(&[U1], &[ALPHA_GAME]),
                        Basis::Inference,
                    )
                }
                .op(),
            ],
            Vec::new(),
        )
    })
    .unwrap();
    h.advance_hours(24 * 400);
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    for i in 0..6 {
        let ev = h.event(&format!("A/n{i}"), "ui-a", ActorKind::User, "x");
        a.record(
            &mut h,
            observation(
                &format!("新的背景观察 {i}"),
                ev,
                &[ALPHA_GAME],
                &format!("A/n{i}/1"),
            ),
        )
        .unwrap();
    }
    let ev = h.event("A/c1", "ui-a", ActorKind::User, "其实可以动");
    a.record(
        &mut h,
        PerceptionInput {
            suggested_kind: Some("correction".into()),
            cites: vec!["item_weak@1".into()],
            ..observation("u1 纠正：不喜欢亮色", ev, &[ALPHA_GAME], "A/c1/1")
        },
    )
    .unwrap();
    let b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let r = h
        .memory
        .query_topic(
            &b.caller,
            &TopicQuery::new(
                b.obs.query_scope().with_title("调整游戏背景效果"),
                Budget::of(4),
            ),
        )
        .unwrap();
    let delivered: Vec<&Hint> = r.hints().collect();
    assert_eq!(delivered.len(), 4);
    assert!(
        delivered[0].state.is_validity_change(),
        "validity changes first: {delivered:?}"
    );
    assert!(
        r.cognitions.iter().any(|x| x.id == "item_c1"),
        "the old explicit boundary keeps its place"
    );
    assert!(!r.perceptions.is_empty(), "new perceptions keep a share");
    assert!(r.truncated && !r.omitted.is_empty());
    let c1 = r.cognitions.iter().find(|x| x.id == "item_c1").unwrap();
    let weak = r.cognitions.iter().find(|x| x.id == "item_weak").unwrap();
    assert_eq!(
        (weak.basis, weak.weight, weak.confidence),
        (Some(Basis::Inference), Some(1.0), Some(0.2))
    );
    assert_eq!(c1.basis, Some(Basis::UserStatement));
    let g = h.memory.graph().unwrap();
    let rec = g
        .recall(&agent_tool::agent_memory::RecallQuery {
            text: Some("背景效果".into()),
            scope: Some(QueryScope {
                subjects: vec![U1.into()],
                objects: vec![ALPHA_GAME.into()],
            }),
            grants: Some(b.caller.grants.clone()),
            now: Some(h.memory.now()),
            ..Default::default()
        })
        .unwrap();
    let parts: BTreeMap<String, f64> = rec
        .items()
        .iter()
        .map(|i| (i.item_id.clone(), i.score_parts.fade))
        .collect();
    assert_eq!(parts["item_c1"], 0.0, "explicit: no fade");
    assert!(parts["item_weak"] > 0.0, "inference fades");
}

#[test]
fn s39_validation_on_the_way_in() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let b = si.begin(&mut h, 5).unwrap();
    let bad_status = plan(
        "bad",
        "bad",
        vec![status_op("item_c1", "bogus", 1, "x")],
        Vec::new(),
    );
    assert!(matches!(
        si.commit(&mut h, &b, bad_status),
        Err(MemoryError::Invalid(_))
    ));
    let e = h.event("A/e2", "ui-a", ActorKind::User, "x");
    let bad_id = plan(
        "bad2",
        "bad",
        vec![obs_op(
            "obs_../../x",
            ObservationKind::Mention,
            "x",
            &e,
            scope(&[U1], &[]),
        )],
        Vec::new(),
    );
    assert!(matches!(
        si.commit(&mut h, &b, bad_id),
        Err(MemoryError::Invalid(_))
    ));
    si.commit(
        &mut h,
        &b,
        plan(
            "del",
            "delete",
            vec![status_op("item_c1", "deleted", 1, "user withdrew it")],
            Vec::new(),
        ),
    )
    .unwrap();
    let revive = plan(
        "revive",
        "revive",
        vec![ItemSpec {
            expected: Some(2),
            ..ItemSpec::new(
                "item_c1",
                "preference",
                "复活",
                &["obs_seed"],
                scope(&[U1], &[ALPHA_GAME]),
                Basis::UserStatement,
            )
        }
        .op()],
        Vec::new(),
    );
    assert!(matches!(
        si.commit(&mut h, &b, revive),
        Err(MemoryError::Conflict(_))
    ));
    let a = h.session("A2", "ui-a2", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e3 = h.event("A/e3", "ui-a2", ActorKind::User, "x");
    assert!(matches!(
        a.record(&mut h, observation("x", e3, &["project:x/../y"], "A/e3/1")),
        Err(MemoryError::Invalid(_))
    ));
    let mut unknown = SourceRef::event(SourceType::SessionEvent, "never/happened");
    unknown.actor_kind = Some(ActorKind::User);
    assert!(matches!(
        a.record(&mut h, observation("x", unknown, &[ALPHA_GAME], "N/1")),
        Err(MemoryError::Invalid(_))
    ));
}

#[test]
fn s40_derived_state_can_disappear() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let root = h.root.join("memory");
    std::fs::remove_file(root.join("memory.sqlite")).unwrap();
    std::fs::remove_dir_all(root.join("index")).unwrap();
    std::fs::remove_dir_all(root.join("item")).unwrap();
    let c = Caller::new("ui-b", &[U1, A1], &[U1]);
    assert_eq!(
        h.memory
            .read_cognition(&c, "item_c1")
            .unwrap()
            .item
            .revision,
        1
    );
    let prov = h.memory.read_provenance(&c, "item_c1").unwrap();
    assert_eq!(prov.current_basis[0].event_ref.as_deref(), Some("A/e1"));
    let q = h
        .memory
        .query(
            &c,
            &MemoryQuery {
                text: Some("背景".into()),
                objects: vec![ALPHA_GAME.into()],
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    assert!(
        matches!(q.index, Some(IndexUse::Scan(_))) && q.cognitions.len() == 1,
        "{q:?}"
    );
    let admin = h.memory.admin().unwrap();
    let rep = admin.verify(false).unwrap();
    assert!(!rep.has_unrecoverable() && !rep.derived_issues.is_empty());
    admin.verify(true).unwrap();
    assert!(admin.verify(false).unwrap().is_clean());
    let q = h
        .memory
        .query(
            &c,
            &MemoryQuery {
                text: Some("背景".into()),
                objects: vec![ALPHA_GAME.into()],
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    assert_eq!(q.index, Some(IndexUse::Fts));
    // A damaged log is reported, never rewritten.
    let log = root.join(".meta/occasions.jsonl");
    let damaged = std::fs::read_to_string(&log)
        .unwrap()
        .replacen("静态", "动态", 1);
    std::fs::write(&log, &damaged).unwrap();
    let rep = admin.verify(true);
    assert!(rep.is_err() || rep.unwrap().has_unrecoverable());
    assert_eq!(std::fs::read_to_string(&log).unwrap(), damaged);
}

#[test]
fn s41_salience_and_observation_text() {
    let (_t, mut h) = host();
    let si = h.si("si");
    let e = h.event(
        "A/e1",
        "ui-a",
        ActorKind::User,
        "演示页的配色由 u1 指定为暗紫",
    );
    let b = si.begin(&mut h, 5).unwrap();
    si.commit(
        &mut h,
        &b,
        plan(
            "sal",
            "object with a salience bump",
            vec![
                obs_op(
                    "obs_demo",
                    ObservationKind::ExplicitStatement,
                    "演示页配色指定为暗紫色",
                    &e,
                    scope(&[U1], &[ALPHA_DEMO]),
                ),
                object_op(
                    "obj_demo",
                    agent_tool::agent_memory::ObjectKind::Concept,
                    "演示页",
                    &[],
                    &["obs_demo"],
                ),
                GraphOperation::ReinforceObjectWeight(ReinforceObjectWeightOp {
                    object_id: "obj_demo".into(),
                    delta: 0.2,
                    reason: "mentioned often".into(),
                    evidence: vec!["obs_demo".into()],
                }),
                ItemSpec::new(
                    "item_demo",
                    "boundary",
                    "u1 指定了演示页配色",
                    &["obs_demo"],
                    scope(&[U1], &[ALPHA_DEMO]),
                    Basis::UserStatement,
                )
                .op(),
            ],
            Vec::new(),
        ),
    )
    .unwrap();
    let c = Caller::new("ui-b", &[U1, A1], &[U1]);
    let q = h
        .memory
        .query(
            &c,
            &MemoryQuery {
                text: Some("暗紫色".into()),
                objects: vec![ALPHA_DEMO.into()],
                ..MemoryQuery::default()
            },
        )
        .unwrap();
    let hit = q
        .cognitions
        .iter()
        .find(|x| x.id == "item_demo")
        .expect("evidence text finds the item");
    assert!(
        hit.reasons.iter().any(|r| r == "fts:observation"),
        "{:?}",
        hit.reasons
    );
    let view = h.memory.graph_view().unwrap();
    assert!(view.items().any(|i| i.kind == ItemKind::Salience));
    assert!(!q
        .cognitions
        .iter()
        .any(|x| x.kind.as_deref() == Some("salience")));
}

#[test]
fn s43_cleanup_and_append_do_not_lose_records() {
    let (_t, mut h) = host();
    let root = h.root.clone();
    let writer = std::thread::spawn(move || {
        let m = plain_memory(&root);
        let lease = session_lease(&root, "ui-w");
        let caller = Caller::new("ui-w", &[U1, A1], &[U1]);
        for i in 0..120 {
            m.record_perception(
                &caller,
                &lease,
                input(
                    &format!("观察 {i}"),
                    &format!("W/e{i}"),
                    &[ALPHA_GAME],
                    &format!("W/e{i}/1"),
                ),
            )
            .unwrap();
        }
    });
    let si = h.si("si");
    let mut disposed = 0;
    while !writer.is_finished() || h.memory.pending_work().unwrap().new > 0 {
        let b = si.begin(&mut h, 15).unwrap();
        if !b.materials.is_empty() {
            let d: Vec<Disposition> = b
                .materials
                .iter()
                .map(|m| discarded(&m.reference, "noise"))
                .collect();
            disposed += d.len();
            si.commit(
                &mut h,
                &b,
                plan(&b.id.clone(), "sweep noise", Vec::new(), d),
            )
            .unwrap();
            si.cleanup(&mut h).unwrap();
        }
    }
    writer.join().unwrap();
    si.cleanup(&mut h).unwrap();
    let recs = read_records(&h.layout().perception_file("ui-w")).unwrap();
    let seqs: Vec<u64> = recs.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        (1..=120).collect::<Vec<_>>(),
        "no record lost, seq never moves back"
    );
    assert_eq!(disposed, 120);
    assert!(recs
        .iter()
        .all(|r| r.cleared.is_some() && r.summary.is_empty() && r.idempotency_key.is_some()));
}

#[test]
fn s44_observing_while_the_graph_writer_lock_is_held() {
    let (_t, mut h) = host();
    let si = h.si("si");
    seed(&mut h, &si);
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(h.root.join("memory/.meta/lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let start = Instant::now();
    for _ in 0..20 {
        assert!(b.observe(&mut h).unwrap().fast_path);
    }
    let w = h.session("W", "ui-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("W/e1", "ui-w", ActorKind::User, "x");
    w.record(&mut h, observation("背景观察", e, &[ALPHA_GAME], "W/e1/1"))
        .unwrap();
    let d = b.observe(&mut h).unwrap();
    assert!(!d.fast_path && d.changes.len() == 1);
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "no lock wait on the read path"
    );
    FileExt::unlock(&lock).unwrap();
}

#[test]
fn s45_runtime_records_feed_consolidation_only() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let w = h.session("W", "work-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let r = w
        .record(
            &mut h,
            PerceptionInput {
                kind: Some("run_digest".into()),
                content: "run 3 ended: background effect adjusted".into(),
                objects: vec![ALPHA_GAME.into()],
                ..PerceptionInput::default()
            },
        )
        .unwrap();
    assert!(b.observe(&mut h).unwrap().changes.is_empty());
    let view = h
        .memory
        .read_perception(&b.caller, r.reference.as_deref().unwrap())
        .unwrap();
    assert_eq!(view.record.kind, "run_digest");
    let si = h.si("si");
    let batch = si.begin(&mut h, 10).unwrap();
    assert_eq!(batch.materials.len(), 1);
    assert!(b
        .topic(&mut h, "调整游戏背景效果", &["运行"])
        .unwrap()
        .result
        .is_none_or(|r| r.perceptions.is_empty()));
}

const CHILD_ROOT: &str = "MEMORY_COMPONENT_CHILD_ROOT";

/// Child entry point: a no-op unless spawned by the two-process test.
#[test]
fn memory_child_writer() {
    let Ok(root) = std::env::var(CHILD_ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let m = plain_memory(&root);
    let lease = session_lease(&root, "proc-child");
    let caller = Caller::new("proc-child", &[U1, A1], &[U1]);
    for i in 0..25 {
        m.record_perception(
            &caller,
            &lease,
            input(
                &format!("子进程观察 {i}"),
                &format!("P/e{i}"),
                &[ALPHA_GAME],
                &format!("P/e{i}/1"),
            ),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn two_processes_share_one_agent() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.budget = Budget::of(100);
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "memory_child_writer",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ROOT, &h.root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The parent writes too, to its own file, while the child writes.
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    for i in 0..5 {
        let e = h.event(&format!("A/e{i}"), "ui-a", ActorKind::User, "x");
        a.record(
            &mut h,
            observation(
                &format!("父进程观察 {i}"),
                e,
                &[ALPHA_GAME],
                &format!("A/e{i}/1"),
            ),
        )
        .unwrap();
    }
    let mut seen = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let d = b.observe(&mut h).unwrap();
        seen.extend(d.changes.into_iter().map(|c| c.hint.id));
        let done = child.try_wait().unwrap();
        if done.is_some() && seen.iter().filter(|s| s.starts_with("proc-child:")).count() == 25 {
            assert!(done.unwrap().success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child output not observed: {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.iter().filter(|s| s.starts_with("ui-a:")).count(), 5);
    let a_state = ObservationState::load(&b.dir.join("observation.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        a_state.snapshot, b.obs.snapshot,
        "each Session keeps its own progress on disk"
    );
    let _ = Arc::new(());
}

#[test]
fn review_resync_from_changes_rebuilds_instead_of_locking_out() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.cfg.max_changes = 2;
    b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    let w = h.session("W", "ui-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    for i in 0..5 {
        let e = h.event(&format!("W/e{i}"), "ui-w", ActorKind::User, "x");
        w.record(
            &mut h,
            observation(
                &format!("背景观察 {i}"),
                e,
                &[ALPHA_GAME],
                &format!("W/e{i}/1"),
            ),
        )
        .unwrap();
    }
    let d = b.observe(&mut h).unwrap();
    assert!(d.resync_required && b.obs.resync_required);
    let out = b.topic(&mut h, "调整游戏背景效果", &[]).unwrap();
    assert_eq!(out.update.valve, Valve::Deep);
    assert_eq!(out.result.unwrap().perceptions.len(), 5);
    assert_eq!(
        b.obs.snapshot,
        Some(h.memory.snapshot().unwrap()),
        "the rebuild adopts the new snapshot"
    );
    let d = b.observe(&mut h).unwrap();
    assert!(!d.resync_required && d.changes.is_empty());
}

#[tokio::test]
async fn review_one_seq_allocator_per_file() {
    let (_t, mut h) = host();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "x");
    a.record(
        &mut h,
        observation("观察一", e.clone(), &[ALPHA_GAME], "A/e1/1"),
    )
    .unwrap();
    // A driver that allocates its own seqs would reuse seq 1: refused, not dropped.
    let client = FsAgentStateClient::open(&h.root, "did:bns:a1", None, None).unwrap();
    let digest = libopendan::state::run_digest(
        "ui-a",
        1,
        "run-1",
        1,
        None,
        &Default::default(),
        Vec::new(),
        "run ended",
        3,
    );
    assert!(client
        .perception()
        .append(&a.lease, "ui-a", vec![digest])
        .await
        .is_err());
    // Explicit seqs: a caller that owns the allocator; identical retries are replays.
    let mut inp = observation("观察二", e.clone(), &[ALPHA_GAME], "A/e1/2");
    inp.seq = Some(5);
    assert_eq!(a.record(&mut h, inp.clone()).unwrap().seq, 5);
    let mut first = observation("观察三", e, &[ALPHA_GAME], "A/e1/3");
    first.seq = Some(7);
    a.record(&mut h, first.clone()).unwrap();
    first.idempotency_key = None;
    let again = a.record(&mut h, first).unwrap();
    assert_eq!((again.status, again.seq), (ReceiptStatus::Replayed, 7));
    let seqs: Vec<u64> = read_records(&h.layout().perception_file("ui-a"))
        .unwrap()
        .iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(seqs, vec![1, 5, 7]);
}

#[test]
fn review_interrupted_append_is_repaired() {
    let (_t, mut h) = host();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "x");
    a.record(
        &mut h,
        observation("观察一", e.clone(), &[ALPHA_GAME], "A/e1/1"),
    )
    .unwrap();
    let path = h.layout().perception_file("ui-a");
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut f, br#"{"seq":2,"at_ms":1,"session_id":"ui-a","ki"#).unwrap();
    drop(f);
    assert!(
        h.memory.snapshot().is_ok(),
        "an unterminated tail does not break snapshots"
    );
    let r = a
        .record(
            &mut h,
            observation("观察二", e.clone(), &[ALPHA_GAME], "A/e1/2"),
        )
        .unwrap();
    assert_eq!(r.seq, 2);
    let recs = read_records(&path).unwrap();
    assert_eq!(recs.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2]);
    // A damaged complete last line: snapshots still work for everyone.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut f, b"{not json}\n").unwrap();
    drop(f);
    let snap = h.memory.snapshot().unwrap();
    assert_eq!(snap.perception_seq("ui-a"), 2);
}

#[test]
fn review_markers_reach_sessions_after_cleanup() {
    let (_t, mut h) = host();
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[]);
    b.topic(&mut h, "动画效果", &["闪烁"]).unwrap();
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[]);
    let e = h.event("A/e1", "ui-a", ActorKind::User, "x");
    let mut inp = observation("用户不喜欢某些效果", e, &[], "A/e1/1");
    inp.tags = vec!["闪烁".into()];
    a.record(&mut h, inp).unwrap();
    let d = b.observe(&mut h).unwrap();
    assert_eq!(change_ids(&d), vec!["ui-a:1".to_string()], "tag match only");
    let si = h.si("si");
    si.run(&mut h, 10, |_, _| {
        plan(
            "drop",
            "drop",
            Vec::new(),
            vec![discarded("ui-a:1", "no later use")],
        )
    })
    .unwrap();
    let d = b.observe(&mut h).unwrap();
    assert!(
        has_change(&d, "ui-a:1", |k| *k == ChangeKind::PerceptionDisposed),
        "the marker arrives although the cleaned record no longer matches by text: {:?}",
        d.changes
    );
}

#[test]
fn review_no_identifier_leaks() {
    let (_t, mut h) = host();
    let si = h.si("si");
    let a = h.session("A", "ui-a", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let e = h.event("A/secret", "ui-a", ActorKind::User, "u1 的私事");
    a.record(
        &mut h,
        observation("u1 的私事", e.clone(), &[ALPHA_GAME], "A/secret/1"),
    )
    .unwrap();
    si.run(&mut h, 10, |_, _| {
        plan(
            "mixed",
            "agent-wide item with private evidence",
            vec![
                obs_op(
                    "obs_private",
                    ObservationKind::ExplicitStatement,
                    "u1 的私事",
                    &e,
                    scope(&[U1], &[ALPHA_GAME]),
                ),
                object_op(
                    "obj_a",
                    agent_tool::agent_memory::ObjectKind::Concept,
                    "A",
                    &[],
                    &["obs_private"],
                ),
                object_op(
                    "obj_b",
                    agent_tool::agent_memory::ObjectKind::Concept,
                    "B",
                    &[],
                    &["obs_private"],
                ),
                GraphOperation::UpsertRelation(agent_tool::agent_memory::UpsertRelationOp {
                    item_id: Some("item_hidden_rel".into()),
                    subject: "obj_a".into(),
                    predicate: "uses".into(),
                    object: "obj_b".into(),
                    weight: 0.5,
                    confidence: 0.5,
                    evidence: vec!["obs_private".into()],
                    write_reason: "private link".into(),
                    replaces: Vec::new(),
                    expected_revision: None,
                    meta: ItemMeta {
                        scope: Some(scope(&[U1], &[])),
                        basis: Some(Basis::UserStatement),
                        ..ItemMeta::default()
                    },
                }),
                ItemSpec {
                    kind: ItemKind::Object,
                    entities: &["obj_a"],
                    ..ItemSpec::new(
                        "item_about_a",
                        "object_context",
                        "A 需要定期检查",
                        &["obs_private"],
                        scope(&[A1], &[]),
                        Basis::Inference,
                    )
                }
                .op(),
                ItemSpec {
                    kind: ItemKind::Object,
                    entities: &["obj_b"],
                    ..ItemSpec::new(
                        "item_about_b",
                        "object_context",
                        "B 需要定期检查",
                        &["obs_private"],
                        scope(&[A1], &[]),
                        Basis::Inference,
                    )
                }
                .op(),
            ],
            vec![absorbed("ui-a:1", &["item_about_a"])],
        )
    })
    .unwrap();
    let g = Caller::new("ui-g", &[U2, A1], &[U2]);
    let read = h.memory.read_cognition(&g, "item_about_a").unwrap();
    assert!(
        read.hint.sources.is_empty(),
        "evidence events of u1 are not listed: {:?}",
        read.hint.sources
    );
    let prov = h.memory.read_provenance(&g, "item_about_a").unwrap();
    assert!(
        prov.absorbed.is_empty(),
        "u1's perception refs are not listed"
    );
    let g_mem = h.memory.graph().unwrap();
    let rec = g_mem
        .recall(&agent_tool::agent_memory::RecallQuery {
            objects: vec!["obj_a".into()],
            grants: Some(g.grants.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        !rec.items().iter().any(|i| i.item_id == "item_about_b"),
        "no expansion through a relation G cannot see"
    );
    let u1 = Caller::new("ui-b", &[U1, A1], &[U1]);
    assert_eq!(
        h.memory
            .read_cognition(&u1, "item_about_a")
            .unwrap()
            .hint
            .sources,
        vec!["A/secret".to_string()]
    );
}
