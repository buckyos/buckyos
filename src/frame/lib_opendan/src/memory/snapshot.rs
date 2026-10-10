//! `memory_snapshot`: a version vector of the Graph log and every perception
//! file (A.9). Writing a perception only moves its own component; cleanup
//! never moves a component; dispositions move `graph_seq`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::state::perception::tail_seq;
use crate::state::AgentLayout;

use super::{Memory, MemoryResult};

/// Opaque to callers (M-24); compare, store and hand back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    graph_seq: u64,
    #[serde(default)]
    perception: BTreeMap<String, u64>,
}

impl MemorySnapshot {
    pub fn graph_seq(&self) -> u64 {
        self.graph_seq
    }

    pub fn perception_seq(&self, sid: &str) -> u64 {
        self.perception.get(sid).copied().unwrap_or(0)
    }

    pub fn perception(&self) -> &BTreeMap<String, u64> {
        &self.perception
    }

    /// Short display form, e.g. `g3/p5`.
    pub fn token(&self) -> String {
        format!(
            "g{}/p{}",
            self.graph_seq,
            self.perception.values().sum::<u64>()
        )
    }
}

/// Session ids that have a perception file.
pub(super) fn perception_sids(layout: &AgentLayout) -> MemoryResult<Vec<String>> {
    let dir = layout.perception_dir();
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(super::MemoryError::Unavailable(format!(
                "{}: {e}",
                dir.display()
            )))
        }
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".jsonl")
                .filter(|s| !s.starts_with('.'))
                .map(str::to_string)
        })
        .collect();
    out.sort();
    Ok(out)
}

pub(super) fn current(m: &Memory) -> MemoryResult<MemorySnapshot> {
    let graph_seq = m.graph()?.graph_seq()?;
    let mut perception = BTreeMap::new();
    for sid in perception_sids(m.layout())? {
        let seq = tail_seq(&m.layout().perception_file(&sid))?;
        if seq > 0 {
            perception.insert(sid, seq);
        }
    }
    Ok(MemorySnapshot {
        graph_seq,
        perception,
    })
}
