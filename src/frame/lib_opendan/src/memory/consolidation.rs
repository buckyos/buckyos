//! The Memory consolidation Goal's side of the component (M-13–M-17,
//! §5.6–§5.9, A.4.1): a cheap pending check without any model call, bounded
//! batches chosen by disposition state (never by a byte cursor), one
//! envelope per commit, and a cleanup that runs and retries on its own.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use agent_tool::agent_memory::{CommitActor, GraphView, ItemStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::{ClearedMarker, PerceptionRecord, RegistryEntry, SessionKind};
use crate::state::perception::clear_records;

use super::perception::parse_cite;
use super::recall::scopes_overlap;
use super::source::SourceEventInfo;
use super::{
    iso, CommitResult, ConsolidationCommit, Disposition, DispositionOutcome, Memory, MemoryError,
    MemoryResult, ProducedBy, CONSOLIDATION_LEASE,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingWork {
    /// Never disposed.
    pub new: usize,
    /// Deferred and past the deadline: must be disposed now.
    pub due: usize,
    /// Deferred, inside the window.
    pub waiting: usize,
    /// Disposed, body not cleaned yet (cleanup to retry).
    pub retry_cleanup: usize,
    pub next_deadline: Option<String>,
}

impl PendingWork {
    /// Worth a consolidation run (new or due material).
    pub fn needs_run(&self) -> bool {
        self.new > 0 || self.due > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialState {
    New,
    Deferred { due: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CitedRef {
    pub cite: String,
    pub current_revision: Option<u64>,
    pub status: Option<ItemStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub reference: String,
    pub record: PerceptionRecord,
    pub state: MaterialState,
    pub disposition: Option<Disposition>,
    /// Cited cognitions with their current revision (echo / correction).
    pub cited: Vec<CitedRef>,
}

/// Materials read in one run. Only these may be disposed by its commit
/// (truncated reads never dispose the unread rest).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub id: String,
    pub materials: Vec<Material>,
    /// Related deferred materials still inside their window (read-only
    /// context; disposing them is allowed since they were read).
    pub context: Vec<Material>,
    pub truncated: bool,
    pub graph_seq: u64,
}

impl Batch {
    pub fn refs(&self) -> BTreeSet<String> {
        self.materials
            .iter()
            .chain(self.context.iter())
            .map(|m| m.reference.clone())
            .collect()
    }

    pub fn material(&self, reference: &str) -> Option<&Material> {
        self.materials
            .iter()
            .chain(self.context.iter())
            .find(|m| m.reference == reference)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchOptions {
    /// Soft limit; one expression (same source event) is never split.
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReport {
    pub cleared: Vec<String>,
    /// `(sid, error)`; retried by the next cleanup.
    pub failed: Vec<(String, String)>,
    pub remaining: usize,
}

/// Holder of the consolidation lease (the only writer of cognitions, M-32).
pub struct Consolidator<'a> {
    memory: &'a Memory,
    lease: &'a Lease,
    session_id: String,
}

fn parse_time(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

struct Classified {
    new: Vec<PerceptionRecord>,
    due: Vec<(PerceptionRecord, Disposition)>,
    waiting: Vec<(PerceptionRecord, Disposition)>,
    retry: BTreeMap<String, BTreeMap<u64, ClearedMarker>>,
    next_deadline: Option<String>,
}

impl Memory {
    fn excluded_session(&self, sid: &str) -> bool {
        fsutil::read_json_opt::<RegistryEntry>(&self.layout().registry_entry(sid))
            .ok()
            .flatten()
            .is_some_and(|e| e.kind == SessionKind::SelfImprove)
    }

    fn classify(&self, view: &GraphView, skip_sid: Option<&str>) -> MemoryResult<Classified> {
        let now = self.now();
        let mut c = Classified {
            new: Vec::new(),
            due: Vec::new(),
            waiting: Vec::new(),
            retry: BTreeMap::new(),
            next_deadline: None,
        };
        let mut excluded: BTreeMap<String, bool> = BTreeMap::new();
        for r in self.load_perceptions(None)? {
            if r.cleared.is_some() {
                continue;
            }
            let skip = *excluded.entry(r.session_id.clone()).or_insert_with(|| {
                skip_sid == Some(r.session_id.as_str()) || self.excluded_session(&r.session_id)
            });
            let reference = r.reference();
            match view.disposition(&reference) {
                None if skip => {}
                None => c.new.push(r),
                Some(d) if d.disposition.outcome.is_terminal() => {
                    c.retry.entry(r.session_id.clone()).or_default().insert(
                        r.seq,
                        ClearedMarker {
                            outcome: d.disposition.outcome.to_string(),
                            occasion_id: d.occasion_id.clone(),
                            cognition_refs: d.disposition.cognition_refs.clone(),
                            at_ms: self.now_ms(),
                        },
                    );
                }
                Some(d) => {
                    let deadline = d.deadline_cap.clone().unwrap_or_default();
                    if parse_time(&deadline).is_some_and(|t| t <= now) {
                        c.due.push((r, d.disposition.clone()));
                    } else {
                        if c.next_deadline.as_ref().is_none_or(|n| &deadline < n) {
                            c.next_deadline = Some(deadline);
                        }
                        c.waiting.push((r, d.disposition.clone()));
                    }
                }
            }
        }
        Ok(c)
    }

    /// Cheap check before giving the consolidation Goal a run (§5.9): no
    /// model call; its own commits are never material.
    pub fn pending_work(&self) -> MemoryResult<PendingWork> {
        let view = self.graph_view()?;
        let c = self.classify(&view, None)?;
        Ok(PendingWork {
            new: c.new.len(),
            due: c.due.len(),
            waiting: c.waiting.len(),
            retry_cleanup: c.retry.values().map(BTreeMap::len).sum(),
            next_deadline: c.next_deadline,
        })
    }

    /// Bind the consolidation lease (A.4.1). Any other lease is refused.
    pub fn consolidator<'a>(
        &'a self,
        lease: &'a Lease,
        session_id: &str,
    ) -> MemoryResult<Consolidator<'a>> {
        if lease.resource() != CONSOLIDATION_LEASE {
            return Err(MemoryError::PermissionDenied(format!(
                "cognitions change only under the {CONSOLIDATION_LEASE} lease, not {}",
                lease.resource()
            )));
        }
        lease.check()?;
        Ok(Consolidator {
            memory: self,
            lease,
            session_id: session_id.to_string(),
        })
    }
}

fn expression_key(r: &PerceptionRecord) -> String {
    match r.source_ref.as_ref().and_then(|s| s.event_ref.clone()) {
        Some(ev) => format!("{}|{ev}", r.session_id),
        None => r.reference(),
    }
}

impl<'a> Consolidator<'a> {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Unrestricted Graph view (the consolidation works for the Agent).
    pub fn graph(&self) -> MemoryResult<Arc<GraphView>> {
        self.memory.graph_view()
    }

    pub fn source(&self, event_ref: &str) -> Option<SourceEventInfo> {
        self.memory
            .config()
            .sources
            .as_ref()
            .and_then(|s| s.lookup(event_ref))
    }

    fn material(
        &self,
        view: &GraphView,
        r: PerceptionRecord,
        state: MaterialState,
        d: Option<Disposition>,
    ) -> Material {
        let cited = r
            .cites
            .iter()
            .map(|c| {
                let item = parse_cite(c)
                    .ok()
                    .and_then(|(id, _)| view.item(&id).cloned());
                CitedRef {
                    cite: c.clone(),
                    current_revision: item.as_ref().map(|i| i.revision),
                    status: item.map(|i| i.status),
                }
            })
            .collect();
        Material {
            reference: r.reference(),
            record: r,
            state,
            disposition: d,
            cited,
        }
    }

    /// Choose a bounded batch: new and due materials in time order, whole
    /// expressions together; related deferred materials as context.
    pub fn begin(&self, opts: &BatchOptions) -> MemoryResult<Batch> {
        self.lease.check()?;
        let view = self.graph()?;
        let c = self.memory.classify(&view, Some(&self.session_id))?;
        let mut candidates: Vec<(PerceptionRecord, MaterialState, Option<Disposition>)> = c
            .new
            .into_iter()
            .map(|r| (r, MaterialState::New, None))
            .chain(
                c.due
                    .into_iter()
                    .map(|(r, d)| (r, MaterialState::Deferred { due: true }, Some(d))),
            )
            .collect();
        candidates.sort_by(|a, b| {
            (a.0.at_ms, &a.0.session_id, a.0.seq).cmp(&(b.0.at_ms, &b.0.session_id, b.0.seq))
        });
        let limit = if opts.limit == 0 { 20 } else { opts.limit };
        let mut groups: Vec<(
            String,
            Vec<(PerceptionRecord, MaterialState, Option<Disposition>)>,
        )> = Vec::new();
        for cand in candidates {
            let key = expression_key(&cand.0);
            match groups.iter_mut().find(|(k, _)| k == &key) {
                Some((_, g)) => g.push(cand),
                None => groups.push((key, vec![cand])),
            }
        }
        let mut materials = Vec::new();
        let mut truncated = false;
        for (_, g) in groups {
            if materials.len() >= limit {
                truncated = true;
                break;
            }
            for (r, s, d) in g {
                materials.push(self.material(&view, r, s, d));
            }
        }
        let context = c
            .waiting
            .into_iter()
            .filter(|(r, _)| {
                materials.iter().any(|m| {
                    scopes_overlap(m.record.scope.as_ref(), r.scope.as_ref())
                        || expression_key(&m.record) == expression_key(r)
                })
            })
            .map(|(r, d)| self.material(&view, r, MaterialState::Deferred { due: false }, Some(d)))
            .collect();
        let ids: Vec<String> = materials
            .iter()
            .map(|m: &Material| m.reference.clone())
            .collect();
        Ok(Batch {
            id: format!(
                "batch-{}",
                &crate::ids::h(&ids.iter().map(String::as_str).collect::<Vec<_>>())[..12]
            ),
            materials,
            context,
            truncated,
            graph_seq: view.seq(),
        })
    }

    /// Commit one consolidation (A.4.1). Only materials of `batch` can be
    /// disposed; deferrals need a window within the configured bound.
    pub fn commit(&self, batch: &Batch, plan: ConsolidationCommit) -> MemoryResult<CommitResult> {
        self.lease.check()?;
        let refs = batch.refs();
        let now = self.memory.now();
        let window = chrono::Duration::from_std(self.memory.config().deferral_window)
            .map_err(|e| MemoryError::Invalid(e.to_string()))?;
        for d in &plan.dispositions {
            if !refs.contains(&d.perception_ref) {
                return Err(MemoryError::Invalid(format!(
                    "{} was not read in this batch; it cannot be disposed",
                    d.perception_ref
                )));
            }
            if d.outcome == DispositionOutcome::Deferred {
                let dl = d.deadline.as_deref().and_then(parse_time).ok_or_else(|| {
                    MemoryError::Invalid(format!("deferred {} needs a deadline", d.perception_ref))
                })?;
                if dl <= now || dl > now + window {
                    return Err(MemoryError::Invalid(format!(
                        "deferral window of {} must end within {} hours",
                        d.perception_ref,
                        window.num_hours()
                    )));
                }
            }
        }
        self.commit_plan(plan)
    }

    fn commit_plan(&self, plan: ConsolidationCommit) -> MemoryResult<CommitResult> {
        let actor = CommitActor {
            session_id: self.session_id.clone(),
            lease_epoch: Some(self.lease.epoch()),
        };
        Ok(self
            .memory
            .graph_writer()?
            .commit_consolidation(&actor, plan)?)
    }

    /// Replace the bodies of disposed perceptions by markers. Independent of
    /// the commit and retried on its own; does not wait for readers.
    pub fn cleanup(&self) -> MemoryResult<CleanupReport> {
        self.lease.check()?;
        let view = self.graph()?;
        let c = self.memory.classify(&view, None)?;
        let mut report = CleanupReport::default();
        for (sid, markers) in c.retry {
            let fail = self
                .memory
                .config()
                .cleanup_fault
                .as_ref()
                .is_some_and(|f| f(&sid));
            match clear_records(
                self.memory.layout(),
                &sid,
                &markers,
                self.memory.config().lock_timeout,
                fail,
            ) {
                Ok(_) => report
                    .cleared
                    .extend(markers.keys().map(|seq| format!("{sid}:{seq}"))),
                Err(e) => {
                    report.remaining += markers.len();
                    report.failed.push((sid, e.to_string()));
                }
            }
        }
        Ok(report)
    }

    /// Fallback sweep (§5.9): deferrals past their deadline plus `grace`
    /// that no run disposed are discarded with the reason kept; the cleared
    /// marker keeps their source.
    pub fn sweep_expired(&self, grace: Duration) -> MemoryResult<Option<CommitResult>> {
        self.lease.check()?;
        let view = self.graph()?;
        let c = self.memory.classify(&view, Some(&self.session_id))?;
        let limit = self.memory.now()
            - chrono::Duration::from_std(grace).map_err(|e| MemoryError::Invalid(e.to_string()))?;
        let expired: Vec<(PerceptionRecord, Disposition)> = c
            .due
            .into_iter()
            .filter(|(_, d)| {
                d.deadline
                    .as_deref()
                    .and_then(parse_time)
                    .is_some_and(|t| t <= limit)
            })
            .collect();
        if expired.is_empty() {
            return Ok(None);
        }
        let refs: Vec<String> = expired.iter().map(|(r, _)| r.reference()).collect();
        let plan = ConsolidationCommit {
            idempotency_key: format!(
                "sweep:{}",
                crate::ids::h(&refs.iter().map(String::as_str).collect::<Vec<_>>())
            ),
            summary: format!(
                "Expired deferrals discarded by the sweep at {}",
                iso(self.memory.now())
            ),
            produced_by: ProducedBy {
                goal_run: Some(format!("sweep:{}", self.session_id)),
                prompt_version: None,
                model: None,
            },
            operations: Vec::new(),
            dispositions: expired
                .into_iter()
                .map(|(r, d)| Disposition {
                    perception_ref: r.reference(),
                    outcome: DispositionOutcome::Discarded,
                    cognition_refs: Vec::new(),
                    reason: Some(format!(
                        "deferral window expired without a decision (was: {})",
                        d.reason.unwrap_or_default()
                    )),
                    reevaluate_when: Vec::new(),
                    deadline: None,
                    clarify: None,
                })
                .collect(),
            tags: Vec::new(),
        };
        self.commit_plan(plan).map(Some)
    }
}
