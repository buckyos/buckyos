//! The owner's view of Memory (Agent UI upgrade TODO, ME1): every cognition
//! and perception of the Agent, newest first, without a Session identity.
//! Only the owner's pages read it: the service has already checked that the
//! caller is the owner, so nothing here filters by grants.
//!
//! One scan of the Graph and the perception files serves the overview and
//! every page; it is kept until the Memory changes (a Graph commit, a new
//! perception, a cleanup).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use agent_tool::agent_memory::{GraphView, ItemKind, ItemStatus, MemoryItem};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::protocol::PerceptionRecord;

use super::snapshot::perception_sids;
use super::{Caller, Memory, MemoryError, MemoryRead, MemoryResult};

/// Cognitions the overview previews (the latest added).
pub const RECENT_COGNITIONS: usize = 5;
/// Perceptions the overview previews (the latest added).
pub const RECENT_PERCEPTIONS: usize = 3;
/// Entries of one page, at most.
pub const MAX_PAGE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLayer {
    Cognition,
    Perception,
}

/// One line of the owner's Memory lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub layer: MemoryLayer,
    /// `item_x@N`, or `<sid>:<seq>`.
    pub reference: String,
    /// The statement of a cognition, the words of a perception (empty once
    /// its body was cleaned).
    pub summary: String,
    pub kind: String,
    /// A cognition: `active | disputed`. A perception: `pending` (never
    /// disposed), `deferred`, `absorbed`, `duplicate`, `discarded`.
    pub state: String,
    /// When it was added: a cognition's first revision, a perception's
    /// record time.
    pub at_ms: u64,
    /// A cognition revised since: when its current revision was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_at_ms: Option<u64>,
    /// A cognition: its current revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    pub subjects: Vec<String>,
    /// A perception: the Session that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The owner may delete it (`forget`): any listed cognition, a
    /// perception no consolidation has settled yet.
    pub deletable: bool,
    /// A settled perception: the cognitions that carry it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absorbed_into: Vec<String>,
    /// A perception whose body was cleaned (only its marker is left).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cleared: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitionCount {
    /// `active` or `disputed`, salience marks left out.
    pub current: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerceptionCount {
    /// Records whose body is still there.
    pub total: usize,
    /// Of those, the ones no consolidation has settled (`pending` or
    /// `deferred`).
    pub pending: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecent {
    pub cognitions: Vec<MemoryEntry>,
    pub perceptions: Vec<MemoryEntry>,
}

/// `memory.overview` without what the consolidation Goal knows (its last
/// run): the service adds that.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOverview {
    pub cognitions: CognitionCount,
    pub perceptions: PerceptionCount,
    pub recent: MemoryRecent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPage {
    pub entries: Vec<MemoryEntry>,
    /// Hand it back for the entries after these; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

struct Scan {
    key: String,
    cognitions: Vec<MemoryEntry>,
    perceptions: Vec<MemoryEntry>,
    counts: (CognitionCount, PerceptionCount),
}

fn scans() -> &'static Mutex<BTreeMap<PathBuf, Arc<Scan>>> {
    static SCANS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Scan>>>> = OnceLock::new();
    SCANS.get_or_init(Default::default)
}

fn iso_to_ms(t: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|t| t.timestamp_millis().max(0) as u64)
}

/// Whether the owner's lists show a cognition: what a recall could return.
fn listed(item: &MemoryItem) -> bool {
    item.status.recallable() && item.kind != ItemKind::Salience
}

fn cognition_entry(view: &GraphView, item: &MemoryItem) -> MemoryEntry {
    let noticed = iso_to_ms(&item.noticed_at);
    let created = view
        .item_created_at(item)
        .as_deref()
        .and_then(iso_to_ms)
        .or(noticed)
        .unwrap_or(0);
    MemoryEntry {
        layer: MemoryLayer::Cognition,
        reference: item.reference(),
        summary: item.statement(),
        kind: item
            .semantic_kind
            .clone()
            .unwrap_or_else(|| item.kind.to_string()),
        state: item.status.to_string(),
        at_ms: created,
        revised_at_ms: noticed.filter(|_| item.revision > 1),
        revision: Some(item.revision),
        subjects: item
            .scope
            .as_ref()
            .map(|s| s.subjects.clone())
            .unwrap_or_default(),
        session_id: None,
        deletable: item.status != ItemStatus::Deleted,
        absorbed_into: Vec::new(),
        cleared: false,
    }
}

fn perception_entry(view: &GraphView, r: &PerceptionRecord) -> MemoryEntry {
    let reference = r.reference();
    let disposition = view.disposition(&reference).map(|d| &d.disposition);
    let (state, absorbed_into, settled) = match (&r.cleared, disposition) {
        (Some(marker), _) => (marker.outcome.clone(), marker.cognition_refs.clone(), true),
        (None, Some(d)) => (
            d.outcome.to_string(),
            d.cognition_refs.clone(),
            d.outcome.is_terminal(),
        ),
        (None, None) => ("pending".to_string(), Vec::new(), false),
    };
    MemoryEntry {
        layer: MemoryLayer::Perception,
        reference,
        summary: r.summary.clone(),
        kind: r.suggested_kind.clone().unwrap_or_else(|| r.kind.clone()),
        state,
        at_ms: r.at_ms,
        revised_at_ms: None,
        revision: None,
        subjects: r
            .scope
            .as_ref()
            .map(|s| s.subjects.clone())
            .unwrap_or_default(),
        session_id: Some(r.session_id.clone()),
        deletable: !settled,
        absorbed_into,
        cleared: r.cleared.is_some(),
    }
}

/// Newest first; entries added in the same millisecond by their reference.
fn order(entries: &mut [MemoryEntry]) {
    entries.sort_by(|a, b| (b.at_ms, &b.reference).cmp(&(a.at_ms, &a.reference)));
}

fn cursor_of(e: &MemoryEntry) -> String {
    format!("{}|{}", e.at_ms, e.reference)
}

fn parse_cursor(cursor: &str) -> MemoryResult<(u64, &str)> {
    cursor
        .split_once('|')
        .and_then(|(at, r)| at.parse::<u64>().ok().map(|at| (at, r)))
        .ok_or_else(|| MemoryError::Invalid(format!("bad cursor `{cursor}`")))
}

impl Memory {
    /// What the scan depends on: the Graph log, every perception file's
    /// tail, and their sizes (a cleanup rewrites a file without moving its
    /// tail).
    fn owner_scan_key(&self) -> MemoryResult<String> {
        let bytes: u64 = perception_sids(self.layout())?
            .iter()
            .filter_map(|sid| std::fs::metadata(self.layout().perception_file(sid)).ok())
            .map(|m| m.len())
            .sum();
        Ok(format!("{}/b{bytes}", self.snapshot()?.token()))
    }

    fn owner_scan(&self) -> MemoryResult<Arc<Scan>> {
        let key = self.owner_scan_key()?;
        let root = self.layout().root.clone();
        if let Some(scan) = scans().lock().expect("memory scans").get(&root) {
            if scan.key == key {
                return Ok(scan.clone());
            }
        }
        let view = self.graph_view()?;
        let mut cognitions: Vec<MemoryEntry> = view
            .items()
            .filter(|i| listed(i))
            .map(|i| cognition_entry(&view, i))
            .collect();
        order(&mut cognitions);
        let mut perceptions: Vec<MemoryEntry> = self
            .load_perceptions(None)?
            .iter()
            .filter(|r| !r.is_runtime())
            .map(|r| perception_entry(&view, r))
            .collect();
        order(&mut perceptions);
        let counts = (
            CognitionCount {
                current: cognitions.len(),
            },
            PerceptionCount {
                total: perceptions.iter().filter(|p| !p.cleared).count(),
                pending: perceptions.iter().filter(|p| p.deletable).count(),
            },
        );
        let scan = Arc::new(Scan {
            key,
            cognitions,
            perceptions,
            counts,
        });
        scans()
            .lock()
            .expect("memory scans")
            .insert(root, scan.clone());
        Ok(scan)
    }

    /// Counts and the latest additions of both layers.
    pub fn owner_overview(&self) -> MemoryResult<MemoryOverview> {
        let scan = self.owner_scan()?;
        Ok(MemoryOverview {
            cognitions: scan.counts.0.clone(),
            perceptions: scan.counts.1.clone(),
            recent: MemoryRecent {
                cognitions: scan.cognitions.iter().take(RECENT_COGNITIONS).cloned().collect(),
                perceptions: scan.perceptions.iter().take(RECENT_PERCEPTIONS).cloned().collect(),
            },
        })
    }

    /// One page of a layer, newest first. The cursor is a position in that
    /// order (the last entry of the page before), so what is written
    /// meanwhile neither repeats an entry nor drops one.
    pub fn owner_list(
        &self,
        layer: MemoryLayer,
        limit: usize,
        cursor: Option<&str>,
    ) -> MemoryResult<MemoryPage> {
        let scan = self.owner_scan()?;
        let all = match layer {
            MemoryLayer::Cognition => &scan.cognitions,
            MemoryLayer::Perception => &scan.perceptions,
        };
        let after = cursor.map(parse_cursor).transpose()?;
        let limit = limit.clamp(1, MAX_PAGE);
        let mut rest = all
            .iter()
            .filter(|e| after.is_none_or(|c| (e.at_ms, e.reference.as_str()) < c));
        let entries: Vec<MemoryEntry> = rest.by_ref().take(limit).cloned().collect();
        let next_cursor = rest.next().and(entries.last()).map(cursor_of);
        Ok(MemoryPage {
            entries,
            next_cursor,
        })
    }

    /// The identity the owner reads as: every subject Memory knows.
    fn owner_caller(&self, view: &GraphView, record: Option<&PerceptionRecord>) -> Caller {
        let mut subjects: Vec<String> = view
            .items()
            .flat_map(|i| view.item_history(&i.item_id).iter())
            .filter_map(|i| i.scope.as_ref())
            .chain(view.observations().filter_map(|o| o.scope.as_ref()))
            .chain(record.and_then(|r| r.scope.as_ref()))
            .flat_map(|s| s.subjects.iter().cloned())
            .collect();
        subjects.sort();
        subjects.dedup();
        Caller::new(
            "owner",
            &subjects.iter().map(String::as_str).collect::<Vec<_>>(),
            &[],
        )
    }

    /// A reference in full, whatever its scope: a cognition (`item_x`,
    /// `item_x@N`) with where it comes from, or a perception
    /// (`<sid>:<seq>`) with its record.
    pub fn owner_read(&self, reference: &str) -> MemoryResult<MemoryRead> {
        let view = self.graph_view()?;
        let record = if reference.starts_with("item_") {
            None
        } else {
            self.perception(reference)?
        };
        let mut caller = self.owner_caller(&view, record.as_ref());
        if reference.starts_with("item_") {
            // The perceptions a cognition absorbed are read too.
            for r in self.load_perceptions(None)? {
                if let Some(s) = &r.scope {
                    caller.grants.extend(s.subjects.iter().cloned());
                }
            }
        }
        self.read(&caller, reference)
    }
}
