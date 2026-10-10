use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, TimeZone};
use serde_json::json;
use tempfile::TempDir;

use super::*;

struct T {
    _tmp: TempDir,
    root: PathBuf,
    now: Arc<AtomicI64>,
    fault: Arc<AtomicU8>,
}

impl T {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("memory");
        Self {
            _tmp: tmp,
            root,
            now: Arc::new(AtomicI64::new(
                Utc.with_ymd_and_hms(2026, 10, 9, 1, 0, 0)
                    .unwrap()
                    .timestamp(),
            )),
            fault: Arc::new(AtomicU8::new(0)),
        }
    }

    fn cfg(&self) -> AgentMemoryConfig {
        let now = self.now.clone();
        let fault = self.fault.clone();
        AgentMemoryConfig::new(&self.root)
            .with_clock(Arc::new(move || {
                Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap()
            }))
            .with_faults(Arc::new(move |p| {
                let f = fault.load(Ordering::SeqCst);
                let hit = matches!(
                    (f, p),
                    (1, FaultPoint::BeforeAppend)
                        | (2, FaultPoint::AppendPartial)
                        | (3, FaultPoint::AppendBeforeFsync)
                        | (4, FaultPoint::BeforeMaterialize)
                );
                if hit {
                    fault.store(0, Ordering::SeqCst);
                }
                hit
            }))
    }

    fn mem(&self) -> AgentMemory {
        AgentMemory::open(self.cfg()).unwrap()
    }

    fn advance_days(&self, days: i64) {
        self.now.fetch_add(days * 86_400, Ordering::SeqCst);
    }

    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.now.load(Ordering::SeqCst), 0)
            .unwrap()
    }
}

fn si() -> CommitActor {
    CommitActor {
        session_id: "si-1".into(),
        lease_epoch: Some(1),
    }
}

fn scope_u1_alpha() -> Scope {
    Scope::new(&["user:u1"], &["project:snake-alpha/game_page"])
}

fn obs(id: &str, content: &str, event: &str) -> GraphOperation {
    GraphOperation::AddObservation(AddObservationOp {
        observation_id: Some(id.into()),
        kind: ObservationKind::ExplicitStatement,
        entities: Vec::new(),
        content: content.into(),
        source_excerpt: None,
        source_ref: Some(SourceRef::event(SourceType::SessionEvent, event)),
        occurred_at: None,
        scope: Some(scope_u1_alpha()),
        confidence: 0.9,
    })
}

fn pref(
    item: &str,
    statement: &str,
    evidence: &[&str],
    expected: Option<u64>,
    scope: Scope,
) -> GraphOperation {
    GraphOperation::PutItem(PutItemOp {
        item_id: Some(item.into()),
        kind: ItemKind::ObservationInference,
        entities: Vec::new(),
        claim: json!({ "type": "observation_inference", "statement": statement }),
        weight: 0.7,
        confidence: 0.8,
        evidence: evidence.iter().map(|s| s.to_string()).collect(),
        write_reason: "user stated a lasting page preference".into(),
        replaces: Vec::new(),
        expected_revision: expected,
        meta: ItemMeta {
            semantic_kind: Some("preference".into()),
            scope: Some(scope),
            basis: Some(Basis::UserStatement),
            explicit: true,
            tags: vec!["背景效果".into()],
            ..ItemMeta::default()
        },
    })
}

fn plan(
    key: &str,
    ops: Vec<GraphOperation>,
    dispositions: Vec<Disposition>,
) -> ConsolidationCommit {
    ConsolidationCommit {
        idempotency_key: key.into(),
        summary: format!("plan {key}"),
        produced_by: ProducedBy {
            goal_run: Some("goal-run-1".into()),
            prompt_version: Some("fixture".into()),
            model: None,
        },
        operations: ops,
        dispositions,
        tags: Vec::new(),
    }
}

fn absorbed(r: &str, c: &str) -> Disposition {
    Disposition {
        perception_ref: r.into(),
        outcome: DispositionOutcome::Absorbed,
        cognition_refs: vec![c.into()],
        reason: None,
        reevaluate_when: Vec::new(),
        deadline: None,
        clarify: None,
    }
}

fn topic_query(t: &T, grants: &[&str], objects: &[&str], text: &str) -> RecallQuery {
    RecallQuery {
        text: Some(text.into()),
        scope: Some(QueryScope {
            subjects: grants.iter().map(|s| s.to_string()).collect(),
            objects: objects.iter().map(|s| s.to_string()).collect(),
        }),
        grants: Some(grants.iter().map(|s| s.to_string()).collect()),
        now: Some(t.now()),
        ..RecallQuery::default()
    }
}

fn seed_c1(t: &T, m: &AgentMemory) -> CommitResult {
    let _ = t;
    m.commit_consolidation(
        &si(),
        plan(
            "batch-1",
            vec![
                obs("obs_1", "u1 要求游戏页静态、不闪烁", "A/e31"),
                obs("obs_2", "静态版不干扰操作，演示页另说", "B2/e12"),
                pref(
                    "item_c1",
                    "u1 在 snake-alpha 游戏页希望默认使用静态、不闪烁的背景，以免干扰操作",
                    &["obs_1", "obs_2"],
                    None,
                    scope_u1_alpha(),
                ),
            ],
            vec![absorbed("A:1", "item_c1"), absorbed("B2:1", "item_c1")],
        ),
    )
    .unwrap()
}

#[test]
fn init_is_idempotent_and_read_open_never_creates() {
    let t = T::new();
    let ro = AgentMemory::open_read(t.cfg()).unwrap();
    assert!(!ro.is_initialized());
    assert_eq!(ro.graph_seq().unwrap(), 0);
    assert_eq!(ro.view().unwrap().seq(), 0);
    assert!(!t.root.exists());
    let _ = t.mem();
    let _ = t.mem();
    assert!(t.root.join(".meta/meta.json").exists());
    assert!(t.root.join("memory.sqlite").exists());
    assert!(matches!(
        ro.commit(&si(), CommitRequest::default()),
        Err(AgentMemoryError::ReadOnly(_)) | Err(AgentMemoryError::Invalid(_))
    ));
}

#[test]
fn free_items_keep_their_logical_id_across_revisions() {
    let t = T::new();
    let m = t.mem();
    m.set("/user/style", "concise", "r").unwrap();
    let id = m
        .view()
        .unwrap()
        .free_item("/user/style")
        .unwrap()
        .item_id
        .clone();
    m.set("/user/style", "concise english", "r").unwrap();
    let v = m.view().unwrap();
    let item = v.free_item("/user/style").unwrap();
    assert_eq!(item.item_id, id);
    assert_eq!(item.revision, 2);
    assert_eq!(v.item_at(&id, 1).unwrap().statement(), "concise");
    m.remove("/user/style", Some("gone")).unwrap();
    assert!(matches!(
        m.get("/user/style"),
        Err(AgentMemoryError::NotFound(_))
    ));
    assert_eq!(
        m.view().unwrap().item(&id).unwrap().status,
        ItemStatus::Deleted
    );
    m.set("/user/style", "verbose", "r").unwrap();
    let v = m.view().unwrap();
    assert_ne!(
        v.free_item("/user/style").unwrap().item_id,
        id,
        "deleted items are not revived"
    );
    assert!(m.set("user/no-slash", "x", "r").is_err());
    assert!(m.set("/.meta/x", "x", "r").is_err());
    assert!(m.set("/a/../b", "x", "r").is_err());
}

#[test]
fn consolidation_needs_evidence_scope_basis_and_returns_real_ids() {
    let t = T::new();
    let m = t.mem();
    let no_ev = m.commit_consolidation(
        &si(),
        plan(
            "k0",
            vec![pref("item_x", "x statement", &[], None, scope_u1_alpha())],
            vec![],
        ),
    );
    assert!(
        matches!(no_ev, Err(AgentMemoryError::Invalid(_))),
        "{no_ev:?}"
    );
    let untraceable = m.commit_consolidation(
        &si(),
        plan(
            "k0",
            vec![GraphOperation::AddObservation(AddObservationOp {
                observation_id: None,
                kind: ObservationKind::ExplicitStatement,
                entities: Vec::new(),
                content: "no source".into(),
                source_excerpt: None,
                source_ref: None,
                occurred_at: None,
                scope: None,
                confidence: 0.5,
            })],
            vec![],
        ),
    );
    assert!(matches!(untraceable, Err(AgentMemoryError::Invalid(_))));
    assert_eq!(m.view().unwrap().seq(), 0, "rejected plans leave nothing");
    let r = seed_c1(&t, &m);
    assert_eq!(r.status, CommitStatus::Committed);
    assert_eq!(
        r.report.observations,
        vec!["obs_1".to_string(), "obs_2".to_string()]
    );
    assert!(r
        .report
        .items
        .iter()
        .any(|c| c.item_id == "item_c1" && c.revision == 1 && c.created));
    let v = m.view().unwrap();
    let c1 = v.item("item_c1").unwrap();
    assert_eq!(c1.basis, Some(Basis::UserStatement));
    assert!(c1.explicit);
    assert_eq!(
        c1.produced_by.as_ref().unwrap().goal_run.as_deref(),
        Some("goal-run-1")
    );
    assert_eq!(
        v.item_source_events(c1, None),
        vec!["A/e31".to_string(), "B2/e12".to_string()]
    );
    assert_eq!(
        v.occasion(&r.occasion_id)
            .unwrap()
            .actor_session_id
            .as_deref(),
        Some("si-1")
    );
    assert_eq!(
        v.disposition("A:1").unwrap().disposition.outcome,
        DispositionOutcome::Absorbed
    );
}

#[test]
fn idempotent_retry_and_conflicting_plans() {
    let t = T::new();
    let m = t.mem();
    let first = seed_c1(&t, &m);
    let again = seed_c1(&t, &m);
    assert_eq!(again.status, CommitStatus::AlreadyCommitted);
    assert_eq!(again.occasion_id, first.occasion_id);
    assert_eq!(m.view().unwrap().seq(), 1);
    let other = m.commit_consolidation(&si(), plan("batch-1", vec![], vec![]));
    assert!(matches!(other, Err(AgentMemoryError::Conflict(_))));
    // A terminal disposition cannot be repeated by another plan.
    let dup = m.commit_consolidation(
        &si(),
        plan("batch-2", vec![], vec![absorbed("A:1", "item_c1")]),
    );
    assert!(matches!(dup, Err(AgentMemoryError::Conflict(_))), "{dup:?}");
    // A pure disposition plan (no operations) is valid.
    let pure = m
        .commit_consolidation(
            &si(),
            plan(
                "batch-3",
                vec![],
                vec![Disposition {
                    perception_ref: "A:2".into(),
                    outcome: DispositionOutcome::Discarded,
                    cognition_refs: Vec::new(),
                    reason: Some("progress copy".into()),
                    reevaluate_when: Vec::new(),
                    deadline: None,
                    clarify: None,
                }],
            ),
        )
        .unwrap();
    assert_eq!(pure.status, CommitStatus::Committed);
}

#[test]
fn revisions_need_the_expected_revision_and_history_is_fixed() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    let stale = m.commit_consolidation(
        &si(),
        plan(
            "rev-a",
            vec![
                obs("obs_3", "可以缓慢移动，但仍不能闪烁", "A/e33"),
                pref(
                    "item_c1",
                    "允许缓慢移动，不闪烁",
                    &["obs_1", "obs_2", "obs_3"],
                    None,
                    scope_u1_alpha(),
                ),
            ],
            vec![],
        ),
    );
    assert!(matches!(stale, Err(AgentMemoryError::Conflict(_))));
    let wrong = m.commit_consolidation(
        &si(),
        plan(
            "rev-b",
            vec![
                obs("obs_3", "可以缓慢移动，但仍不能闪烁", "A/e33"),
                pref(
                    "item_c1",
                    "允许缓慢移动，不闪烁",
                    &["obs_3"],
                    Some(7),
                    scope_u1_alpha(),
                ),
            ],
            vec![],
        ),
    );
    assert!(matches!(wrong, Err(AgentMemoryError::Conflict(_))));
    m.commit_consolidation(
        &si(),
        plan(
            "rev-c",
            vec![
                obs("obs_3", "可以缓慢移动，但仍不能闪烁", "A/e33"),
                pref(
                    "item_c1",
                    "允许缓慢移动，不闪烁",
                    &["obs_1", "obs_2", "obs_3"],
                    Some(1),
                    scope_u1_alpha(),
                ),
            ],
            vec![absorbed("A:3", "item_c1")],
        ),
    )
    .unwrap();
    let v = m.view().unwrap();
    assert_eq!(v.item("item_c1").unwrap().revision, 2);
    assert!(v
        .item_at("item_c1", 1)
        .unwrap()
        .statement()
        .contains("静态"));
    assert!(v.item("item_c1").unwrap().statement().contains("缓慢移动"));
    assert_eq!(m.get_item_json("item_c1@1").unwrap().contains("静态"), true);
}

#[test]
fn relation_triples_revise_in_scope_and_stay_apart_across_scopes() {
    let t = T::new();
    let m = t.mem();
    let objects = |key: &str| {
        plan(
            key,
            vec![
                obs("obs_r", "u1 prefers static on alpha", "A/e1"),
                GraphOperation::UpsertObject(UpsertObjectOp {
                    object_id: Some("obj_u1".into()),
                    kind: ObjectKind::User,
                    canonical_name: "u1".into(),
                    aliases: vec![ObjectAliasInput {
                        alias: "user:u1".into(),
                        alias_type: AliasType::Did,
                        confidence: 1.0,
                    }],
                    evidence: vec!["obs_r".into()],
                    weight: None,
                    confidence: 1.0,
                    merge_into: None,
                }),
                GraphOperation::UpsertObject(UpsertObjectOp {
                    object_id: Some("obj_static".into()),
                    kind: ObjectKind::Concept,
                    canonical_name: "static background".into(),
                    aliases: Vec::new(),
                    evidence: vec!["obs_r".into()],
                    weight: None,
                    confidence: 1.0,
                    merge_into: None,
                }),
            ],
            vec![],
        )
    };
    m.commit_consolidation(&si(), objects("objs")).unwrap();
    let rel = |key: &str, scope: Scope, expected: Option<u64>| {
        plan(
            key,
            vec![GraphOperation::UpsertRelation(UpsertRelationOp {
                item_id: None,
                subject: "obj_u1".into(),
                predicate: "prefers".into(),
                object: "obj_static".into(),
                weight: 0.6,
                confidence: 0.7,
                evidence: vec!["obs_r".into()],
                write_reason: "stated".into(),
                replaces: Vec::new(),
                expected_revision: expected,
                meta: ItemMeta {
                    scope: Some(scope),
                    basis: Some(Basis::UserStatement),
                    ..ItemMeta::default()
                },
            })],
            vec![],
        )
    };
    let a = m
        .commit_consolidation(&si(), rel("r1", scope_u1_alpha(), None))
        .unwrap();
    let id_a = a.report.items[0].item_id.clone();
    assert!(matches!(
        m.commit_consolidation(&si(), rel("r2", scope_u1_alpha(), None)),
        Err(AgentMemoryError::Conflict(_))
    ));
    let a2 = m
        .commit_consolidation(&si(), rel("r3", scope_u1_alpha(), Some(1)))
        .unwrap();
    assert_eq!(a2.report.items[0].item_id, id_a);
    assert_eq!(a2.report.items[0].revision, 2);
    let beta = Scope::new(&["user:u1"], &["project:snake-beta"]);
    let b = m
        .commit_consolidation(&si(), rel("r4", beta, None))
        .unwrap();
    assert_ne!(
        b.report.items[0].item_id, id_a,
        "other project scope is another cognition"
    );
}

#[test]
fn status_transitions_are_checked() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    let set = |key: &str, status: &str, expected: u64, evidence: Vec<String>| {
        m.commit(
            &si(),
            CommitRequest {
                occasion_type: "consolidation".into(),
                summary: "status".into(),
                operations: vec![GraphOperation::SetStatus(SetStatusOp {
                    target_kind: TargetKind::Item,
                    target_id: "item_c1".into(),
                    status: status.into(),
                    reason: "fixture".into(),
                    replaced_by: None,
                    expected_revision: Some(expected),
                    evidence,
                })],
                idempotency_key: Some(key.into()),
                produced_by: Some(ProducedBy {
                    goal_run: Some("g".into()),
                    ..ProducedBy::default()
                }),
                consolidation: true,
                ..CommitRequest::default()
            },
        )
    };
    assert!(matches!(
        set("s0", "bogus", 1, vec![]),
        Err(AgentMemoryError::Invalid(_))
    ));
    set("s1", "superseded", 1, vec![]).unwrap();
    assert!(matches!(
        set("s2", "active", 2, vec![]),
        Err(AgentMemoryError::Conflict(_))
    ));
    set("s3", "active", 2, vec!["obs_1".into()]).unwrap();
    set("s4", "deleted", 3, vec![]).unwrap();
    assert!(matches!(
        set("s5", "active", 4, vec!["obs_1".into()]),
        Err(AgentMemoryError::Conflict(_))
    ));
}

#[test]
fn deferral_windows_cannot_be_extended() {
    let t = T::new();
    let m = t.mem();
    let deferred = |deadline: &str| Disposition {
        perception_ref: "W:1".into(),
        outcome: DispositionOutcome::Deferred,
        cognition_refs: Vec::new(),
        reason: Some("no verification yet".into()),
        reevaluate_when: vec!["direct verification".into()],
        deadline: Some(deadline.into()),
        clarify: None,
    };
    m.commit_consolidation(
        &si(),
        plan("d1", vec![], vec![deferred("2026-10-12T01:00:00Z")]),
    )
    .unwrap();
    assert!(matches!(
        m.commit_consolidation(
            &si(),
            plan("d2", vec![], vec![deferred("2026-10-20T01:00:00Z")])
        ),
        Err(AgentMemoryError::Conflict(_))
    ));
    let mut missing = deferred("2026-10-12T01:00:00Z");
    missing.reevaluate_when.clear();
    assert!(m
        .commit_consolidation(&si(), plan("d3", vec![], vec![missing]))
        .is_err());
}

#[test]
fn lease_epochs_fence_stale_writers() {
    let t = T::new();
    let m = t.mem();
    m.commit_consolidation(
        &CommitActor {
            session_id: "si-2".into(),
            lease_epoch: Some(5),
        },
        plan("e5", vec![], vec![]),
    )
    .unwrap();
    let old = m.commit_consolidation(
        &CommitActor {
            session_id: "si-1".into(),
            lease_epoch: Some(4),
        },
        plan("e4", vec![], vec![]),
    );
    assert!(matches!(old, Err(AgentMemoryError::StaleLease(_))));
}

#[test]
fn recall_filters_scope_and_visibility_and_reads_chinese_full_text() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    let none = m
        .recall(&RecallQuery {
            grants: Some(["user:u1".to_string()].into()),
            ..RecallQuery::default()
        })
        .unwrap();
    assert_eq!(none, RecallOutcome::NotTriggered);
    assert!(matches!(
        m.recall(&RecallQuery {
            tags: vec!["bad\"tag".into()],
            ..RecallQuery::default()
        }),
        Err(AgentMemoryError::Invalid(_))
    ));
    let hit = m
        .recall(&topic_query(
            &t,
            &["user:u1", "agent:a1"],
            &["project:snake-alpha/game_page"],
            "调整游戏背景效果",
        ))
        .unwrap();
    let items = hit.items();
    assert_eq!(items.len(), 1);
    assert!(items[0].matched.iter().any(|m| m.starts_with("scope:")));
    assert!(items[0].matched.iter().any(|m| m == "fts:item"));
    assert!(matches!(
        hit,
        RecallOutcome::Recalled(RecallResult {
            index: IndexUse::Fts,
            ..
        })
    ));
    // Project-level query sees the page-level cognition.
    assert_eq!(
        m.recall(&topic_query(
            &t,
            &["user:u1"],
            &["project:snake-alpha"],
            "背景"
        ))
        .unwrap()
        .items()
        .len(),
        1
    );
    // Other project: relevance filter.
    assert!(m
        .recall(&topic_query(
            &t,
            &["user:u1"],
            &["project:snake-beta/game_page"],
            "调整游戏背景效果"
        ))
        .unwrap()
        .items()
        .is_empty());
    // u2 on the same page: visibility filter.
    assert!(m
        .recall(&topic_query(
            &t,
            &["user:u2", "agent:a1"],
            &["project:snake-alpha/game_page"],
            "调整游戏背景效果"
        ))
        .unwrap()
        .items()
        .is_empty());
    // `snake-alpha2` does not overlap `snake-alpha`.
    assert!(m
        .recall(&topic_query(
            &t,
            &["user:u1"],
            &["project:snake-alpha2"],
            "背景"
        ))
        .unwrap()
        .items()
        .is_empty());
    // Without the FTS cache the scan returns the same items.
    fs::remove_file(t.root.join(SQLITE_FILE)).unwrap();
    let scan = m
        .recall(&topic_query(
            &t,
            &["user:u1"],
            &["project:snake-alpha/game_page"],
            "调整游戏背景效果",
        ))
        .unwrap();
    assert_eq!(scan.items().len(), 1);
    assert!(matches!(
        scan,
        RecallOutcome::Recalled(RecallResult {
            index: IndexUse::Scan(_),
            ..
        })
    ));
}

#[test]
fn explicit_cognitions_do_not_fade_and_validity_is_enforced() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    let q = || topic_query(&t, &["user:u1"], &["project:snake-alpha/game_page"], "背景");
    let before = m.recall(&q()).unwrap().items()[0].score;
    t.advance_days(400);
    let after = m.recall(&q()).unwrap().items()[0].clone();
    assert_eq!(after.score_parts.fade, 0.0);
    assert_eq!(after.score, before);
    let mut fq = q();
    fq.fade = FadeConfig {
        days_per_point: 1.0,
        max_penalty: 3.0,
    };
    assert_eq!(m.recall(&fq).unwrap().items()[0].score_parts.fade, 0.0);
    // valid_until in the past filters a cognition even if explicit.
    m.commit_consolidation(
        &si(),
        plan(
            "valid",
            vec![GraphOperation::PutItem(PutItemOp {
                item_id: Some("item_tmp".into()),
                kind: ItemKind::ObservationInference,
                entities: Vec::new(),
                claim: json!({ "type": "observation_inference", "statement": "临时背景活动规则" }),
                weight: 0.9,
                confidence: 0.9,
                evidence: vec!["obs_1".into()],
                write_reason: "fixture".into(),
                replaces: Vec::new(),
                expected_revision: None,
                meta: ItemMeta {
                    scope: Some(scope_u1_alpha()),
                    basis: Some(Basis::UserStatement),
                    explicit: true,
                    valid_until: Some((t.now() + ChronoDuration::hours(1)).to_rfc3339()),
                    ..ItemMeta::default()
                },
            })],
            vec![],
        ),
    )
    .unwrap();
    assert!(m
        .recall(&q())
        .unwrap()
        .items()
        .iter()
        .any(|i| i.item_id == "item_tmp"));
    t.now.fetch_add(7200, Ordering::SeqCst);
    assert!(!m
        .recall(&q())
        .unwrap()
        .items()
        .iter()
        .any(|i| i.item_id == "item_tmp"));
}

#[test]
fn aliases_report_ambiguity_and_merges_redirect() {
    let t = T::new();
    let m = t.mem();
    let bob = |id: &str, name: &str| {
        GraphOperation::UpsertObject(UpsertObjectOp {
            object_id: Some(id.into()),
            kind: ObjectKind::Person,
            canonical_name: name.into(),
            aliases: vec![ObjectAliasInput {
                alias: "Bob".into(),
                alias_type: AliasType::Name,
                confidence: 0.8,
            }],
            evidence: vec!["obs_b".into()],
            weight: None,
            confidence: 0.8,
            merge_into: None,
        })
    };
    let r = m
        .commit_consolidation(
            &si(),
            plan(
                "bobs",
                vec![
                    obs("obs_b", "two contacts are called Bob", "K/e1"),
                    bob("obj_bob_pm", "Bob (product manager)"),
                    bob("obj_bob_nb", "Bob (neighbour)"),
                ],
                vec![],
            ),
        )
        .unwrap();
    assert_eq!(
        r.report.objects[1].shared_aliases[0].other_objects,
        vec!["obj_bob_pm".to_string()]
    );
    let amb = m
        .recall(&RecallQuery {
            aliases: vec!["bob".into()],
            ..RecallQuery::default()
        })
        .unwrap();
    match amb {
        RecallOutcome::Recalled(r) => assert_eq!(r.ambiguous_aliases[0].candidates.len(), 2),
        other => panic!("{other:?}"),
    }
    // Matching by alias without an id refuses to pick one.
    let pick = m.commit_consolidation(
        &si(),
        plan(
            "pick",
            vec![GraphOperation::UpsertObject(UpsertObjectOp {
                object_id: None,
                kind: ObjectKind::Person,
                canonical_name: "Bob".into(),
                aliases: vec![ObjectAliasInput {
                    alias: "bob".into(),
                    alias_type: AliasType::Name,
                    confidence: 0.5,
                }],
                evidence: vec!["obs_b".into()],
                weight: None,
                confidence: 0.5,
                merge_into: None,
            })],
            vec![],
        ),
    );
    assert!(matches!(pick, Err(AgentMemoryError::Ambiguous { .. })));
    // Confirmed merge: the old object redirects.
    m.commit_consolidation(
        &si(),
        plan(
            "merge",
            vec![GraphOperation::UpsertObject(UpsertObjectOp {
                object_id: Some("obj_bob_nb".into()),
                kind: ObjectKind::Person,
                canonical_name: "Bob".into(),
                aliases: Vec::new(),
                evidence: vec!["obs_b".into()],
                weight: None,
                confidence: 0.9,
                merge_into: Some("obj_bob_pm".into()),
            })],
            vec![],
        ),
    )
    .unwrap();
    let v = m.view().unwrap();
    assert_eq!(v.resolve_object("obj_bob_nb"), "obj_bob_pm");
    assert_eq!(v.resolve_alias("bob"), vec!["obj_bob_pm".to_string()]);
    assert!(v
        .object("obj_bob_nb")
        .unwrap()
        .aliases
        .iter()
        .all(|a| a.status == AliasStatus::Merged));
}

#[test]
fn relation_hops_expand_from_query_objects() {
    let t = T::new();
    let m = t.mem();
    let object = |id: &str| {
        GraphOperation::UpsertObject(UpsertObjectOp {
            object_id: Some(id.into()),
            kind: ObjectKind::Concept,
            canonical_name: id.into(),
            aliases: Vec::new(),
            evidence: vec!["obs_h".into()],
            weight: None,
            confidence: 0.9,
            merge_into: None,
        })
    };
    let rel = |s: &str, o: &str| {
        GraphOperation::UpsertRelation(UpsertRelationOp {
            item_id: None,
            subject: s.into(),
            predicate: "uses".into(),
            object: o.into(),
            weight: 0.5,
            confidence: 0.5,
            evidence: vec!["obs_h".into()],
            write_reason: "fixture".into(),
            replaces: Vec::new(),
            expected_revision: None,
            meta: ItemMeta {
                scope: Some(Scope::new(&["agent:a1"], &[])),
                basis: Some(Basis::ToolObservation),
                ..ItemMeta::default()
            },
        })
    };
    m.commit_consolidation(
        &si(),
        plan(
            "hops",
            vec![
                obs("obs_h", "chain", "V/c1"),
                object("obj_a"),
                object("obj_b"),
                object("obj_c"),
                rel("obj_a", "obj_b"),
                rel("obj_b", "obj_c"),
                GraphOperation::PutItem(PutItemOp {
                    item_id: Some("item_about_c".into()),
                    kind: ItemKind::Object,
                    entities: vec!["obj_c".into()],
                    claim: json!({ "type": "object", "object_id": "obj_c", "statement": "c needs a check" }),
                    weight: 0.5,
                    confidence: 0.5,
                    evidence: vec!["obs_h".into()],
                    write_reason: "fixture".into(),
                    replaces: Vec::new(),
                    expected_revision: None,
                    meta: ItemMeta {
                        scope: Some(Scope::new(&["agent:a1"], &[])),
                        basis: Some(Basis::ToolObservation),
                        ..ItemMeta::default()
                    },
                }),
            ],
            vec![],
        ),
    )
    .unwrap();
    let r = m
        .recall(&RecallQuery {
            objects: vec!["obj_a".into()],
            grants: Some(["agent:a1".to_string()].into()),
            ..RecallQuery::default()
        })
        .unwrap();
    let about_c = r
        .items()
        .iter()
        .find(|i| i.item_id == "item_about_c")
        .expect("2-hop item");
    assert!(about_c.matched.iter().any(|m| m.starts_with("hop2:obj_c")));
    let none = m
        .recall(&RecallQuery {
            objects: vec!["obj_a".into()],
            grants: Some(["agent:a1".to_string()].into()),
            expand_hops: 0,
            ..RecallQuery::default()
        })
        .unwrap();
    assert!(!none.items().iter().any(|i| i.item_id == "item_about_c"));
}

#[test]
fn salience_and_lost_evidence_stay_out_of_recall() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    m.commit(
        &si(),
        CommitRequest {
            occasion_type: "consolidation".into(),
            summary: "evidence withdrawn".into(),
            operations: vec![GraphOperation::SetStatus(SetStatusOp {
                target_kind: TargetKind::Observation,
                target_id: "obs_2".into(),
                status: "deleted".into(),
                reason: "source retracted".into(),
                replaced_by: None,
                expected_revision: None,
                evidence: Vec::new(),
            })],
            idempotency_key: Some("ev".into()),
            produced_by: Some(ProducedBy {
                goal_run: Some("g".into()),
                ..ProducedBy::default()
            }),
            consolidation: true,
            ..CommitRequest::default()
        },
    )
    .unwrap();
    assert!(m
        .recall(&topic_query(
            &t,
            &["user:u1"],
            &["project:snake-alpha/game_page"],
            "背景"
        ))
        .unwrap()
        .items()
        .is_empty());
}

#[test]
fn injected_failures_keep_the_log_honest() {
    let t = T::new();
    let m = t.mem();
    t.fault.store(1, Ordering::SeqCst);
    assert!(matches!(seed_c1_try(&m), Err(AgentMemoryError::Io(_))));
    assert_eq!(m.graph_seq().unwrap(), 0);

    t.fault.store(2, Ordering::SeqCst);
    assert!(matches!(
        seed_c1_try(&m),
        Err(AgentMemoryError::CommitUnknown(_))
    ));
    // Readers ignore the torn tail; the retry drops it and commits once.
    assert_eq!(
        AgentMemory::open_read(t.cfg())
            .unwrap()
            .view()
            .unwrap()
            .seq(),
        0
    );
    let r = seed_c1_try(&m).unwrap();
    assert!(r.recovered_torn_tail);
    assert_eq!(r.status, CommitStatus::Committed);
    assert_eq!(m.view().unwrap().seq(), 1);

    t.fault.store(3, Ordering::SeqCst);
    let p = plan("after-fsync", vec![], vec![absorbed("C:1", "item_c1")]);
    assert!(matches!(
        m.commit_consolidation(&si(), p.clone()),
        Err(AgentMemoryError::CommitUnknown(_))
    ));
    let again = m.commit_consolidation(&si(), p).unwrap();
    assert_eq!(again.status, CommitStatus::AlreadyCommitted);
    assert_eq!(m.view().unwrap().seq(), 2);

    t.fault.store(4, Ordering::SeqCst);
    let r = m
        .commit_consolidation(&si(), plan("mat", vec![], vec![absorbed("C:2", "item_c1")]))
        .unwrap();
    assert!(matches!(r.derived, DerivedStatus::PendingRepair(_)));
    let rep = m.verify(false).unwrap();
    assert!(!rep.has_unrecoverable());
    assert!(!rep.derived_issues.is_empty());
    assert!(m.repair_derived().unwrap());
    assert!(m.verify(false).unwrap().is_clean());
}

fn seed_c1_try(m: &AgentMemory) -> Result<CommitResult> {
    m.commit_consolidation(
        &si(),
        plan(
            "batch-1",
            vec![
                obs("obs_1", "u1 要求游戏页静态、不闪烁", "A/e31"),
                pref("item_c1", "静态背景", &["obs_1"], None, scope_u1_alpha()),
            ],
            vec![absorbed("A:1", "item_c1")],
        ),
    )
}

#[test]
fn corruption_is_reported_not_repaired() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    m.set("/k", "v", "r").unwrap();
    let log = m.log_path();
    let text = fs::read_to_string(&log).unwrap();
    let tampered = text.replacen("静态", "动态", 1);
    fs::write(&log, tampered).unwrap();
    let rep = m.verify(false).unwrap();
    assert!(rep.has_unrecoverable(), "{rep:?}");
    let fresh = AgentMemory::open_read(t.cfg()).unwrap();
    assert!(matches!(fresh.view(), Err(AgentMemoryError::Corrupted(_))));
    assert!(matches!(
        fresh.recall(&RecallQuery {
            tags: vec!["背景".into()],
            ..RecallQuery::default()
        }),
        Err(AgentMemoryError::Corrupted(_))
    ));
    fs::write(&log, text).unwrap();
    assert!(AgentMemory::open_read(t.cfg()).unwrap().view().is_ok());
}

#[test]
fn compact_archives_with_a_manifest() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    m.set("/user/a", "alpha", "r").unwrap();
    m.compact().unwrap();
    assert_eq!(fs::metadata(m.log_path()).unwrap().len(), 0);
    assert_eq!(m.graph_seq().unwrap(), 2);
    assert_eq!(m.get("/user/a").unwrap(), "alpha");
    m.set("/user/b", "beta", "r").unwrap();
    assert_eq!(m.graph_seq().unwrap(), 3);
    assert_eq!(m.view().unwrap().item("item_c1").unwrap().revision, 1);
    assert!(m.verify(false).unwrap().is_clean());
    let archive = t.root.join(".meta/archive");
    let file = fs::read_dir(&archive)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .unwrap();
    fs::remove_file(file).unwrap();
    assert!(matches!(
        AgentMemory::open_read(t.cfg()).unwrap().view(),
        Err(AgentMemoryError::Corrupted(_))
    ));
}

#[test]
fn reads_do_not_take_the_writer_lock() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(t.root.join(".meta/lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let mut cfg = t.cfg();
    cfg.lock_timeout = Duration::from_millis(100);
    let r = AgentMemory::open_read(cfg.clone()).unwrap();
    assert_eq!(r.view().unwrap().seq(), 1);
    assert!(r.graph_seq().is_ok());
    let w = AgentMemory::open(cfg).unwrap();
    assert!(matches!(
        w.set("/x", "y", "r"),
        Err(AgentMemoryError::LockTimeout(_))
    ));
    FileExt::unlock(&lock).unwrap();
}

#[test]
fn schema_rules_and_explicit_migration() {
    let t = T::new();
    migrate::write_legacy_root(&t.root).unwrap();
    assert!(matches!(
        AgentMemory::open(t.cfg()),
        Err(AgentMemoryError::NeedsMigration(_))
    ));
    assert!(matches!(
        AgentMemory::open_read(t.cfg()),
        Err(AgentMemoryError::NeedsMigration(_))
    ));
    let report = AgentMemory::migrate(t.cfg()).unwrap();
    assert_eq!(report.from, "2.10");
    assert_eq!(report.occasions, 2);
    assert!(
        report.notes.iter().any(|n| n.contains("relationship")),
        "{:?}",
        report.notes
    );
    assert!(
        report.notes.iter().any(|n| n.contains("notebook")),
        "{:?}",
        report.notes
    );
    let m = t.mem();
    assert_eq!(m.get("/user/style").unwrap(), "concise");
    assert_eq!(
        m.view().unwrap().object("obj_user").unwrap().kind,
        ObjectKind::User
    );
    assert!(t.root.join(".meta/legacy-2.10/occasions.jsonl").exists());
    assert!(m.verify(false).unwrap().is_clean());
    // A newer minor mounts read-only.
    let meta_path = t.root.join(".meta/meta.json");
    let mut meta: Value = serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
    meta["schema_version"] = json!("3.9");
    fs::write(&meta_path, serde_json::to_vec(&meta).unwrap()).unwrap();
    let ro = AgentMemory::open(t.cfg()).unwrap();
    assert!(ro.read_only_reason().is_some());
    assert_eq!(ro.get("/user/style").unwrap(), "concise");
    assert!(matches!(
        ro.set("/x", "y", "r"),
        Err(AgentMemoryError::ReadOnly(_))
    ));
}

#[test]
fn path_segments_are_encoded() {
    assert_eq!(percent_segment(".."), "%2E%2E");
    assert_eq!(percent_segment("a/b"), "a%2Fb");
    let t = T::new();
    let m = t.mem();
    let bad = m.commit_consolidation(
        &si(),
        plan("bad-id", vec![obs("obs_../../x", "x", "E/1")], vec![]),
    );
    assert!(matches!(bad, Err(AgentMemoryError::Invalid(_))));
}

#[test]
fn legacy_load_output_has_the_new_fields() {
    let t = T::new();
    let m = t.mem();
    m.set_free(FlatSetOp {
        key: "/user/dental".into(),
        content: "Dental followup at 10am".into(),
        reason: "r".into(),
        entities: Vec::new(),
        weight: None,
        confidence: None,
        evidence: Vec::new(),
        expected_revision: None,
        meta: ItemMeta {
            tags: vec!["dental".into()],
            ..ItemMeta::default()
        },
    })
    .unwrap();
    let items = m.load(&["dental".into()], LoadOptions::default()).unwrap();
    assert_eq!(items.len(), 1);
    let s = AgentMemory::format_load_items(&items);
    for line in [
        "REVISION 1\n",
        "KIND free\n",
        "STATE active\n",
        "MATCHED tag:dental",
        "---\nDental",
    ] {
        assert!(s.contains(line), "{s}");
    }
    assert!(
        m.load(&[], LoadOptions::default()).unwrap().is_empty(),
        "no entry, no whole-graph dump"
    );
}

#[test]
fn deadlines_compare_as_times_not_strings() {
    let t = T::new();
    let m = t.mem();
    let deferred = |deadline: &str| Disposition {
        perception_ref: "W:1".into(),
        outcome: DispositionOutcome::Deferred,
        cognition_refs: Vec::new(),
        reason: Some("waiting".into()),
        reevaluate_when: vec!["verification".into()],
        deadline: Some(deadline.into()),
        clarify: None,
    };
    m.commit_consolidation(
        &si(),
        plan("d1", vec![], vec![deferred("2026-10-12T01:00:00Z")]),
    )
    .unwrap();
    // 00:30Z written with an offset: earlier, so it is a legal shortening.
    m.commit_consolidation(
        &si(),
        plan("d2", vec![], vec![deferred("2026-10-12T08:30:00+08:00")]),
    )
    .unwrap();
    assert_eq!(
        m.view()
            .unwrap()
            .disposition("W:1")
            .unwrap()
            .deadline_cap
            .as_deref(),
        Some("2026-10-12T00:30:00Z")
    );
    // 01:30Z written with an offset: later than the cap.
    assert!(matches!(
        m.commit_consolidation(
            &si(),
            plan("d3", vec![], vec![deferred("2026-10-12T09:30:00+08:00")])
        ),
        Err(AgentMemoryError::Conflict(_))
    ));
}

#[test]
fn replay_tolerates_the_compaction_window() {
    let t = T::new();
    let m = t.mem();
    seed_c1(&t, &m);
    m.set("/user/a", "alpha", "r").unwrap();
    let lines = fs::read(m.log_path()).unwrap();
    m.compact().unwrap();
    // The live log still holding the archived lines (mid-compaction).
    fs::write(m.log_path(), &lines).unwrap();
    let r = AgentMemory::open_read(t.cfg()).unwrap();
    assert_eq!(r.view().unwrap().seq(), 2);
    assert_eq!(r.get("/user/a").unwrap(), "alpha");
    assert_eq!(r.graph_seq().unwrap(), 2);
    // A different envelope under the same seq is still corruption.
    fs::write(m.log_path(), b"").unwrap();
    m.set("/user/b", "beta", "r").unwrap();
    let mut forged: MemoryOccasion = serde_json::from_slice(
        fs::read(m.log_path())
            .unwrap()
            .split(|b| *b == b'\n')
            .next()
            .unwrap(),
    )
    .unwrap();
    forged.seq = 2;
    forged.digest = occasion_digest(&forged).unwrap();
    let mut line = serde_json::to_vec(&forged).unwrap();
    line.push(b'\n');
    fs::write(m.log_path(), line).unwrap();
    assert!(matches!(
        AgentMemory::open_read(t.cfg()).unwrap().view(),
        Err(AgentMemoryError::Corrupted(_))
    ));
}

#[test]
fn migration_resumes_after_a_crash() {
    let t = T::new();
    migrate::write_legacy_root(&t.root).unwrap();
    AgentMemory::migrate(t.cfg()).unwrap();
    // Crash before the new meta landed: the old meta is back in place.
    fs::copy(
        t.root.join(".meta/legacy-2.10/meta.json"),
        t.root.join(".meta/meta.json"),
    )
    .unwrap();
    let again = AgentMemory::migrate(t.cfg()).unwrap();
    assert_eq!(again.occasions, 2);
    let m = t.mem();
    assert_eq!(m.get("/user/style").unwrap(), "concise");
    assert!(m.view().unwrap().object("obj_user").is_some());
}
