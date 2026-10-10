//! Simulation host for the Memory component stage (C).
//!
//! `SimSession` is a light Session: identity and grants, a real Session
//! lease, its `ObservationState` persisted in its own directory, and the
//! Memory material that would enter its next model input. `SimSi` holds the
//! real consolidation lease and commits preset plans. Storage, matching,
//! commits, notifications and recovery all run the real component; only
//! the model's judgements (what to write, the consolidation plan) are
//! fixtures. No Runner, LLM, tool or message bus is started.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use agent_tool::agent_memory::{FaultPoint, GraphView, ObservationKind};
use chrono::{DateTime, FixedOffset, Utc};
use libopendan::lock::{Acquire, Lease};
use libopendan::memory::preview::{render_changes, render_topic, Names};
use libopendan::memory::*;
use libopendan::protocol::HolderInfo;
use libopendan::state::AgentLayout;
use serde::Serialize;
use serde_json::{json, Value};

pub const U1: &str = "user:u1";
pub const U2: &str = "user:u2";
pub const A1: &str = "agent:a1";
pub const ALPHA: &str = "project:snake-alpha";
pub const ALPHA_GAME: &str = "project:snake-alpha/game_page";
pub const ALPHA_DEMO: &str = "project:snake-alpha/demo_page";
pub const BETA: &str = "project:snake-beta";
pub const BETA_GAME: &str = "project:snake-beta/game_page";

// ---------------------------------------------------------------------------
// source events (fixture) and trace
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Events {
    map: Mutex<BTreeMap<String, SourceEventInfo>>,
}

impl SourceEvents for Events {
    fn lookup(&self, event_ref: &str) -> Option<SourceEventInfo> {
        self.map.lock().unwrap().get(event_ref).cloned()
    }
}

impl Events {
    pub fn add(&self, ev: &str, actor: ActorKind, text: &str, at: &str) {
        self.map.lock().unwrap().insert(
            ev.to_string(),
            SourceEventInfo {
                event_ref: ev.to_string(),
                actor_kind: Some(actor),
                readable: true,
                excerpt: Some(text.to_string()),
                observed_at: Some(at.to_string()),
                runtime_attachment: false,
            },
        );
    }

    /// A Memory block the Runtime attached to a model input.
    pub fn add_attachment(&self, ev: &str) {
        self.map.lock().unwrap().insert(
            ev.to_string(),
            SourceEventInfo {
                event_ref: ev.to_string(),
                actor_kind: Some(ActorKind::Runtime),
                readable: true,
                excerpt: None,
                observed_at: None,
                runtime_attachment: true,
            },
        );
    }

    /// The original history was purged.
    pub fn purge(&self, ev: &str) {
        if let Some(e) = self.map.lock().unwrap().get_mut(ev) {
            e.readable = false;
            e.excerpt = None;
        }
    }

    pub fn text(&self, ev: &str) -> String {
        self.map
            .lock()
            .unwrap()
            .get(ev)
            .and_then(|e| e.excerpt.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Default)]
pub struct Trace {
    pub lines: Vec<String>,
    pub checks: Vec<Check>,
    pub verbose: bool,
}

impl Trace {
    fn push(&mut self, line: String) {
        if self.verbose {
            println!("{line}");
        }
        self.lines.push(line);
    }

    pub fn section(&mut self, title: &str) {
        self.push(String::new());
        self.push(format!("════ {title}"));
    }

    pub fn step(&mut self, at: &str, who: &str, what: &str) {
        self.push(format!("── {at} {who}｜{what}"));
    }

    pub fn note(&mut self, text: &str) {
        self.push(format!("   {text}"));
    }

    pub fn call(&mut self, call: &str, result: &str) {
        self.push(format!("   call   {call}"));
        self.push(format!("   result {result}"));
    }

    pub fn json<T: Serialize>(&mut self, label: &str, v: &T) {
        let s = serde_json::to_string(v).unwrap_or_default();
        let s: String = s.chars().take(600).collect();
        self.push(format!("   {label} {s}"));
    }

    pub fn history(&mut self, who: &str, block: &str) {
        self.push(format!("   ┌ 下一次模型输入中 {who} 的 Memory 材料"));
        for l in block.lines() {
            self.push(format!("   │ {l}"));
        }
        self.push("   └".to_string());
    }

    pub fn check(&mut self, id: &str, ok: bool, detail: impl Into<String>) {
        let detail = detail.into();
        self.push(format!("   {} {id} {detail}", if ok { "✓" } else { "✗" }));
        self.checks.push(Check {
            id: id.to_string(),
            ok,
            detail,
        });
    }

    pub fn failed(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| !c.ok).collect()
    }
}

// ---------------------------------------------------------------------------
// host
// ---------------------------------------------------------------------------

pub struct Host {
    pub root: PathBuf,
    pub clock: Arc<ManualClock>,
    pub events: Arc<Events>,
    pub memory: Memory,
    pub names: Names,
    pub trace: Trace,
    pub cleanup_fail: Arc<Mutex<BTreeSet<String>>>,
    /// 0 none, 1 before append, 2 partial append, 3 after write, 4 materialize.
    pub graph_fault: Arc<AtomicU8>,
    /// World objects (project selection, …): not Memory.
    pub world: BTreeMap<String, Value>,
    tz: FixedOffset,
}

fn holder(name: &str) -> HolderInfo {
    HolderInfo {
        runner_id: format!("sim-{name}"),
        principal: name.to_string(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    }
}

impl Host {
    pub fn new(root: &Path, start_local: &str) -> Self {
        let start = DateTime::parse_from_rfc3339(start_local).expect("start time");
        let clock = ManualClock::at(start.with_timezone(&Utc));
        let events = Arc::new(Events::default());
        let cleanup_fail = Arc::new(Mutex::new(BTreeSet::new()));
        let graph_fault = Arc::new(AtomicU8::new(0));
        let memory = Self::make_memory(root, &clock, &events, &cleanup_fail, &graph_fault);
        Self {
            root: root.to_path_buf(),
            clock,
            events,
            memory,
            names: Names::default(),
            trace: Trace::default(),
            cleanup_fail,
            graph_fault,
            world: BTreeMap::new(),
            tz: *start.offset(),
        }
    }

    fn make_memory(
        root: &Path,
        clock: &Arc<ManualClock>,
        events: &Arc<Events>,
        cleanup_fail: &Arc<Mutex<BTreeSet<String>>>,
        graph_fault: &Arc<AtomicU8>,
    ) -> Memory {
        let fail = cleanup_fail.clone();
        let fault = graph_fault.clone();
        Memory::open(
            root,
            MemoryConfig {
                clock: clock.clone(),
                sources: Some(events.clone()),
                cleanup_fault: Some(Arc::new(move |sid: &str| fail.lock().unwrap().remove(sid))),
                graph_faults: Some(Arc::new(move |p: FaultPoint| {
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
                })),
                ..MemoryConfig::default()
            },
        )
    }

    /// A process restart: a fresh component instance on the same files.
    pub fn restart(&mut self) {
        self.memory = Self::make_memory(
            &self.root,
            &self.clock,
            &self.events,
            &self.cleanup_fail,
            &self.graph_fault,
        );
    }

    pub fn layout(&self) -> AgentLayout {
        AgentLayout::new(&self.root)
    }

    pub fn at(&mut self, local: &str) {
        let t = DateTime::parse_from_rfc3339(local).expect("time");
        self.clock.set(t.with_timezone(&Utc));
    }

    pub fn advance_hours(&mut self, h: u64) {
        self.clock.advance(std::time::Duration::from_secs(h * 3600));
    }

    pub fn now_local(&self) -> String {
        self.memory
            .now()
            .with_timezone(&self.tz)
            .format("%m-%d %H:%M")
            .to_string()
    }

    pub fn now_iso_local(&self) -> String {
        self.memory.now().with_timezone(&self.tz).to_rfc3339()
    }

    /// Deadline `hours` from now (UTC RFC 3339).
    pub fn after_hours(&self, hours: i64) -> String {
        iso(self.memory.now() + chrono::Duration::hours(hours))
    }

    pub fn name(&mut self, id: &str, name: &str) {
        self.names.0.insert(id.to_string(), name.to_string());
    }

    pub fn n(&self, id: &str) -> String {
        self.names.name(id)
    }

    /// Register an original event and return its source reference.
    pub fn event(&mut self, ev: &str, session: &str, actor: ActorKind, text: &str) -> SourceRef {
        let at = self.now_iso_local();
        self.events.add(ev, actor, text, &at);
        let mut s = SourceRef::event(
            if actor == ActorKind::Tool {
                SourceType::ToolResult
            } else {
                SourceType::SessionEvent
            },
            ev,
        );
        s.session_id = Some(session.to_string());
        s.actor_kind = Some(actor);
        s
    }

    pub fn session(
        &mut self,
        name: &str,
        sid: &str,
        grants: &[&str],
        subjects: &[&str],
        objects: &[&str],
    ) -> SimSession {
        SimSession::open(self, name, sid, grants, subjects, objects)
    }

    pub fn si(&self, sid: &str) -> SimSi {
        let layout = self.layout();
        let path = layout.lock_path(CONSOLIDATION_LEASE).unwrap();
        let lease = match Lease::acquire(CONSOLIDATION_LEASE, &path, holder(sid)).unwrap() {
            Acquire::Acquired(l) => l,
            Acquire::Busy(_) => panic!("consolidation lease busy"),
        };
        SimSi {
            sid: sid.to_string(),
            lease,
        }
    }

    pub fn check(&mut self, id: &str, ok: bool, detail: impl Into<String>) {
        self.trace.check(id, ok, detail);
    }

    /// Text of every perception file (to prove what is not in Memory).
    pub fn perception_files(h: &Host) -> String {
        let mut out = String::new();
        if let Ok(rd) = std::fs::read_dir(h.layout().perception_dir()) {
            for e in rd.flatten() {
                out.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

pub struct SimSession {
    pub name: String,
    pub sid: String,
    pub dir: PathBuf,
    pub lease: Lease,
    pub caller: Caller,
    pub obs: ObservationState,
    pub history: Vec<String>,
    pub cfg: TopicConfig,
    pub budget: Budget,
}

impl SimSession {
    fn open(
        host: &mut Host,
        name: &str,
        sid: &str,
        grants: &[&str],
        subjects: &[&str],
        objects: &[&str],
    ) -> Self {
        let dir = host.root.join("sim-sessions").join(sid);
        std::fs::create_dir_all(&dir).unwrap();
        let lease = match Lease::acquire(
            &format!("session:{sid}"),
            &dir.join("lease.json"),
            holder(name),
        )
        .unwrap()
        {
            Acquire::Acquired(l) => l,
            Acquire::Busy(_) => panic!("session {sid} busy"),
        };
        let path = dir.join("observation.json");
        let obs = ObservationState::load(&path).unwrap().unwrap_or_else(|| {
            ObservationState::new(QueryScope {
                subjects: grants.iter().map(|s| s.to_string()).collect(),
                objects: objects.iter().map(|s| s.to_string()).collect(),
            })
        });
        Self {
            name: name.to_string(),
            sid: sid.to_string(),
            dir,
            lease,
            caller: Caller::new(sid, grants, subjects),
            obs,
            history: Vec::new(),
            cfg: TopicConfig::default(),
            budget: Budget::default(),
        }
    }

    /// Restart the session process: the state comes back from its file.
    pub fn reopen(self, host: &mut Host) -> Self {
        let (name, sid) = (self.name.clone(), self.sid.clone());
        let grants: Vec<String> = self.caller.grants.iter().cloned().collect();
        let subjects = self.caller.default_subjects.clone();
        let cfg = self.cfg.clone();
        let budget = self.budget.clone();
        drop(self);
        let g: Vec<&str> = grants.iter().map(String::as_str).collect();
        let s: Vec<&str> = subjects.iter().map(String::as_str).collect();
        let mut me = SimSession::open(host, &name, &sid, &g, &s, &[]);
        me.cfg = cfg;
        me.budget = budget;
        me
    }

    pub fn save(&self) {
        self.obs.save(&self.dir.join("observation.json")).unwrap();
    }

    pub fn record(
        &self,
        host: &mut Host,
        input: PerceptionInput,
    ) -> MemoryResult<PerceptionReceipt> {
        let r = host
            .memory
            .record_perception(&self.caller, &self.lease, input);
        match &r {
            Ok(rc) => host.trace.call(
                &format!("{}.memory.record_perception(...)", self.name),
                &format!(
                    "{:?} {} related={:?}",
                    rc.status,
                    rc.reference
                        .as_deref()
                        .map(|r| host.n(r))
                        .unwrap_or_default(),
                    rc.related_cognitions
                        .iter()
                        .map(|c| host.n(c.split('@').next().unwrap_or(c)))
                        .collect::<Vec<_>>()
                ),
            ),
            Err(e) => host.trace.call(
                &format!("{}.memory.record_perception(...)", self.name),
                &format!("error: {e}"),
            ),
        }
        r
    }

    /// `set_topic`, then assemble and accept the recall (when the valve
    /// opened and assembly succeeds).
    pub fn topic(
        &mut self,
        host: &mut Host,
        title: &str,
        tags: &[&str],
    ) -> MemoryResult<TopicOutcome> {
        self.topic_with(host, title, tags, true)
    }

    pub fn topic_with(
        &mut self,
        host: &mut Host,
        title: &str,
        tags: &[&str],
        assemble_ok: bool,
    ) -> MemoryResult<TopicOutcome> {
        let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let out = set_topic(
            &host.memory,
            &self.caller,
            &mut self.obs,
            title,
            &tags,
            &self.budget,
            &self.cfg,
        )?;
        host.trace.call(
            &format!("{}.set_topic({title:?}, tags={tags:?})", self.name),
            &format!(
                "topic r{} valve={:?} status={:?} cognitions={:?} perceptions={:?} snapshot={}",
                out.update.revision,
                out.update.valve,
                out.result.as_ref().map(|r| r.status),
                out.result
                    .as_ref()
                    .map(|r| r
                        .cognitions
                        .iter()
                        .map(|h| format!(
                            "{}@{}:{:?}",
                            host.n(&h.id),
                            h.revision.unwrap_or(0),
                            h.state
                        ))
                        .collect::<Vec<_>>())
                    .unwrap_or_default(),
                out.result
                    .as_ref()
                    .map(|r| r
                        .perceptions
                        .iter()
                        .map(|h| host.n(&h.id))
                        .collect::<Vec<_>>())
                    .unwrap_or_default(),
                out.result
                    .as_ref()
                    .map(|r| r.snapshot.token())
                    .unwrap_or_default(),
            ),
        );
        if let Some(r) = &out.result {
            if r.status == RecallStatus::Recalled && assemble_ok {
                let block = render_topic(&host.names, title, r);
                if r.hints().next().is_some() {
                    host.trace.history(&self.name, &block);
                    self.history.push(block);
                }
                self.obs
                    .accept_recall(r, &self.caller, host.memory.now_ms());
                self.save();
            }
        }
        Ok(out)
    }

    pub fn observe(&mut self, host: &mut Host) -> MemoryResult<Delivery> {
        let budget = self.budget.clone();
        self.observe_with(host, &budget, true)
    }

    /// One observation opportunity before an inference. `assemble_ok =
    /// false` simulates a failed assembly: nothing is accepted.
    pub fn observe_with(
        &mut self,
        host: &mut Host,
        budget: &Budget,
        assemble_ok: bool,
    ) -> MemoryResult<Delivery> {
        let d = observe(&host.memory, &self.caller, &self.obs, budget, &self.cfg)?;
        host.trace.call(
            &format!("{}.observe(budget={})", self.name, budget.max_items),
            &format!(
                "changes={:?} overflow={:?} fast_path={} disabled={} resync={}",
                d.changes
                    .iter()
                    .map(|c| format!("{}:{}", kind_label(&c.kind), host.n(&c.hint.id)))
                    .collect::<Vec<_>>(),
                d.overflow,
                d.fast_path,
                d.disabled,
                d.resync_required
            ),
        );
        if assemble_ok && !d.disabled {
            if !d.changes.is_empty() {
                let block = render_changes(&host.names, &self.obs.topic.title, &d.changes);
                host.trace.history(&self.name, &block);
                self.history.push(block);
            }
            self.obs.accept(&d, &self.cfg);
            self.save();
        }
        Ok(d)
    }

    pub fn last_history(&self) -> &str {
        self.history.last().map(String::as_str).unwrap_or("")
    }
}

pub fn kind_label(k: &ChangeKind) -> String {
    match k {
        ChangeKind::NewPerception => "new_perception".into(),
        ChangeKind::PerceptionDisposed => "disposed".into(),
        ChangeKind::NewCognition => "new_cognition".into(),
        ChangeKind::RevisedCognition { from } => format!("revised_from@{from}"),
        ChangeKind::CognitionStatus { status } => format!("status:{status}"),
        ChangeKind::ReviewPending { .. } => "review_pending".into(),
        ChangeKind::ReadRevision { from } => format!("read_revision_from@{from}"),
    }
}

pub struct SimSi {
    pub sid: String,
    pub lease: Lease,
}

impl SimSi {
    pub fn begin(&self, host: &mut Host, limit: usize) -> MemoryResult<Batch> {
        let c = host.memory.consolidator(&self.lease, &self.sid)?;
        let b = c.begin(&BatchOptions { limit })?;
        host.trace.call(
            &format!("SI.begin_consolidation(limit={limit})"),
            &format!(
                "{} materials={:?} context={:?} truncated={}",
                b.id,
                b.materials
                    .iter()
                    .map(|m| host.n(&m.reference))
                    .collect::<Vec<_>>(),
                b.context
                    .iter()
                    .map(|m| host.n(&m.reference))
                    .collect::<Vec<_>>(),
                b.truncated
            ),
        );
        Ok(b)
    }

    pub fn commit(
        &self,
        host: &mut Host,
        batch: &Batch,
        plan: ConsolidationCommit,
    ) -> MemoryResult<CommitResult> {
        let c = host.memory.consolidator(&self.lease, &self.sid)?;
        let key = plan.idempotency_key.clone();
        let r = c.commit(batch, plan);
        match &r {
            Ok(r) => host.trace.call(
                &format!("SI.commit_consolidation(key={key})"),
                &format!(
                    "{:?} {} items={:?} derived={:?}",
                    r.status,
                    r.occasion_id,
                    r.report
                        .items
                        .iter()
                        .map(|i| format!("{}@{}", host.n(&i.item_id), i.revision))
                        .collect::<Vec<_>>(),
                    r.derived
                ),
            ),
            Err(e) => host.trace.call(
                &format!("SI.commit_consolidation(key={key})"),
                &format!("error: {e}"),
            ),
        }
        r
    }

    pub fn cleanup(&self, host: &mut Host) -> MemoryResult<CleanupReport> {
        let c = host.memory.consolidator(&self.lease, &self.sid)?;
        let r = c.cleanup()?;
        host.trace.call(
            "SI.cleanup_consolidated()",
            &format!(
                "cleared={:?} failed={:?} remaining={}",
                r.cleared.iter().map(|x| host.n(x)).collect::<Vec<_>>(),
                r.failed,
                r.remaining
            ),
        );
        Ok(r)
    }

    pub fn graph(&self, host: &Host) -> Arc<GraphView> {
        host.memory.graph_view().unwrap()
    }

    /// One whole run: batch, preset plan, commit, cleanup.
    pub fn run(
        &self,
        host: &mut Host,
        limit: usize,
        plan: impl FnOnce(&Batch, &GraphView) -> ConsolidationCommit,
    ) -> MemoryResult<(Batch, CommitResult)> {
        let b = self.begin(host, limit)?;
        let p = plan(&b, &self.graph(host));
        let r = self.commit(host, &b, p)?;
        self.cleanup(host)?;
        Ok((b, r))
    }
}

// ---------------------------------------------------------------------------
// plan builders (the "preset decisions")
// ---------------------------------------------------------------------------

pub fn scope(subjects: &[&str], objects: &[&str]) -> Scope {
    Scope::new(subjects, objects)
}

pub fn obs_op(
    id: &str,
    kind: ObservationKind,
    content: &str,
    source: &SourceRef,
    scope: Scope,
) -> GraphOperation {
    GraphOperation::AddObservation(agent_tool::agent_memory::AddObservationOp {
        observation_id: Some(id.to_string()),
        kind,
        entities: Vec::new(),
        content: content.to_string(),
        source_excerpt: None,
        source_ref: Some(source.clone()),
        occurred_at: None,
        scope: Some(scope),
        confidence: 0.9,
    })
}

pub fn object_op(
    id: &str,
    kind: agent_tool::agent_memory::ObjectKind,
    name: &str,
    aliases: &[(&str, agent_tool::agent_memory::AliasType)],
    evidence: &[&str],
) -> GraphOperation {
    GraphOperation::UpsertObject(agent_tool::agent_memory::UpsertObjectOp {
        object_id: Some(id.to_string()),
        kind,
        canonical_name: name.to_string(),
        aliases: aliases
            .iter()
            .map(|(a, t)| agent_tool::agent_memory::ObjectAliasInput {
                alias: a.to_string(),
                alias_type: *t,
                confidence: 0.9,
            })
            .collect(),
        evidence: evidence.iter().map(|s| s.to_string()).collect(),
        weight: None,
        confidence: 0.9,
        merge_into: None,
    })
}

/// A cognition written by a preset plan.
pub struct ItemSpec<'a> {
    pub id: &'a str,
    pub kind: ItemKind,
    pub semantic: &'a str,
    pub statement: &'a str,
    pub evidence: &'a [&'a str],
    pub expected: Option<u64>,
    pub scope: Scope,
    pub basis: Basis,
    pub explicit: bool,
    pub tags: &'a [&'a str],
    pub entities: &'a [&'a str],
    pub review_when: &'a [&'a str],
    pub weight: f64,
    pub confidence: f64,
    pub valid_until: Option<String>,
}

impl<'a> ItemSpec<'a> {
    pub fn new(
        id: &'a str,
        semantic: &'a str,
        statement: &'a str,
        evidence: &'a [&'a str],
        scope: Scope,
        basis: Basis,
    ) -> Self {
        Self {
            id,
            kind: ItemKind::ObservationInference,
            semantic,
            statement,
            evidence,
            expected: None,
            scope,
            basis,
            explicit: false,
            tags: &[],
            entities: &[],
            review_when: &[],
            weight: 0.6,
            confidence: 0.7,
            valid_until: None,
        }
    }

    pub fn op(&self) -> GraphOperation {
        let claim = match self.kind {
            ItemKind::Attribute => json!({
                "type": "attribute",
                "subject": self.entities.first().copied().unwrap_or("obj_u1"),
                "attribute": self.semantic,
                "value": self.statement,
                "statement": self.statement,
            }),
            ItemKind::EventEffect => json!({
                "type": "event_effect",
                "affected_objects": self.entities,
                "effect": self.statement,
                "statement": self.statement,
            }),
            ItemKind::Object => json!({
                "type": "object",
                "object_id": self.entities.first().copied().unwrap_or(""),
                "statement": self.statement,
            }),
            k => json!({ "type": k.as_str(), "statement": self.statement }),
        };
        GraphOperation::PutItem(agent_tool::agent_memory::PutItemOp {
            item_id: Some(self.id.to_string()),
            kind: self.kind,
            entities: self.entities.iter().map(|s| s.to_string()).collect(),
            claim,
            weight: self.weight,
            confidence: self.confidence,
            evidence: self.evidence.iter().map(|s| s.to_string()).collect(),
            write_reason: format!("{} worth reusing", self.semantic),
            replaces: Vec::new(),
            expected_revision: self.expected,
            meta: ItemMeta {
                semantic_kind: Some(self.semantic.to_string()),
                scope: Some(self.scope.clone()),
                basis: Some(self.basis),
                explicit: self.explicit,
                valid_until: self.valid_until.clone(),
                review_when: self.review_when.iter().map(|s| s.to_string()).collect(),
                tags: self.tags.iter().map(|s| s.to_string()).collect(),
            },
        })
    }
}

pub fn status_op(item: &str, status: &str, expected: u64, reason: &str) -> GraphOperation {
    GraphOperation::SetStatus(agent_tool::agent_memory::SetStatusOp {
        target_kind: agent_tool::agent_memory::TargetKind::Item,
        target_id: item.to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        replaced_by: None,
        expected_revision: Some(expected),
        evidence: Vec::new(),
    })
}

fn disposition(r: &str, outcome: DispositionOutcome) -> Disposition {
    Disposition {
        perception_ref: r.to_string(),
        outcome,
        cognition_refs: Vec::new(),
        reason: None,
        reevaluate_when: Vec::new(),
        deadline: None,
        clarify: None,
    }
}

pub fn absorbed(r: &str, cognitions: &[&str]) -> Disposition {
    Disposition {
        cognition_refs: cognitions.iter().map(|s| s.to_string()).collect(),
        ..disposition(r, DispositionOutcome::Absorbed)
    }
}

pub fn duplicate(r: &str, cognition: Option<&str>, reason: &str) -> Disposition {
    Disposition {
        cognition_refs: cognition.into_iter().map(str::to_string).collect(),
        reason: Some(reason.to_string()),
        ..disposition(r, DispositionOutcome::Duplicate)
    }
}

pub fn discarded(r: &str, reason: &str) -> Disposition {
    Disposition {
        reason: Some(reason.to_string()),
        ..disposition(r, DispositionOutcome::Discarded)
    }
}

pub fn deferred(
    r: &str,
    reason: &str,
    when: &[&str],
    deadline: &str,
    clarify: Option<&str>,
) -> Disposition {
    Disposition {
        reason: Some(reason.to_string()),
        reevaluate_when: when.iter().map(|s| s.to_string()).collect(),
        deadline: Some(deadline.to_string()),
        clarify: clarify.map(str::to_string),
        ..disposition(r, DispositionOutcome::Deferred)
    }
}

pub fn plan(
    key: &str,
    summary: &str,
    operations: Vec<GraphOperation>,
    dispositions: Vec<Disposition>,
) -> ConsolidationCommit {
    ConsolidationCommit {
        idempotency_key: key.to_string(),
        summary: summary.to_string(),
        produced_by: ProducedBy {
            goal_run: Some(format!("goal-run:{key}")),
            prompt_version: Some("fixture-plan".into()),
            model: None,
        },
        operations,
        dispositions,
        tags: Vec::new(),
    }
}

/// A plain observation input.
pub fn observation(
    content: &str,
    source: SourceRef,
    objects: &[&str],
    key: &str,
) -> PerceptionInput {
    PerceptionInput {
        content: content.to_string(),
        objects: objects.iter().map(|s| s.to_string()).collect(),
        source: Some(source),
        idempotency_key: Some(key.to_string()),
        ..PerceptionInput::default()
    }
}

pub fn hint_ids(hints: &[Hint]) -> Vec<String> {
    hints.iter().map(|h| h.id.clone()).collect()
}

pub fn change_ids(d: &Delivery) -> Vec<String> {
    d.changes.iter().map(|c| c.hint.id.clone()).collect()
}

pub fn has_change(d: &Delivery, id: &str, f: impl Fn(&ChangeKind) -> bool) -> bool {
    d.changes.iter().any(|c| c.hint.id == id && f(&c.kind))
}
