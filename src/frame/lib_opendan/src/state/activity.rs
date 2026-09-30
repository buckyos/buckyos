//! Active session view (§6.7). A hint for avoidance, never a lock.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::registry::FsRegistry;
use super::{ActivityView, SessionRegistry};

/// `running` entries whose heartbeat is older than this are shown as
/// "possibly interrupted"; `waiting` needs no heartbeat.
pub const STALE_HEARTBEAT_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Same workspace or same artifact.
    SameTarget,
    /// Touching sets intersect.
    Overlap,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveSession {
    pub session_id: String,
    pub kind: SessionKind,
    pub run_state: RunState,
    pub objective: String,
    pub activity: Activity,
    pub relation: Relation,
    /// Refs shared with the caller's scope / touching.
    pub overlap: Vec<String>,
    pub possibly_interrupted: bool,
    pub driver: String,
}

pub(super) struct FsActivity {
    registry: FsRegistry,
}

impl FsActivity {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self {
            registry: FsRegistry::new(layout, None, None),
        }
    }
}

/// Normalized refs a session declares: scope paths / objects, workspace,
/// artifact and activity touching.
pub fn declared_refs(e: &RegistryEntry) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    if let Some(s) = &e.scope {
        refs.extend(s.paths.iter().cloned());
        refs.extend(s.objects.iter().cloned());
    }
    for t in &e.status.activity.touching {
        refs.push(t.target.clone());
    }
    if let Some(a) = &e.artifact_id {
        refs.push(format!("artifact:{a}"));
    }
    refs.sort();
    refs.dedup();
    refs
}

/// Two refs overlap when equal or when one is a path prefix of the other
/// (`ws:snake/src/` vs `ws:snake/src/collision.js`).
pub fn refs_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (a, b) = (a.trim_end_matches('/'), b.trim_end_matches('/'));
    a == b || b.starts_with(&format!("{a}/")) || a.starts_with(&format!("{b}/"))
}

fn same_target(me: &RegistryEntry, other: &RegistryEntry) -> bool {
    (me.workspace.is_some() && me.workspace == other.workspace)
        || (me.artifact_id.is_some() && me.artifact_id == other.artifact_id)
}

#[async_trait]
impl ActivityView for FsActivity {
    async fn active(&self, me: Option<&RegistryEntry>, limit: usize) -> Result<Vec<ActiveSession>> {
        let now = crate::now_ms();
        let entries = self
            .registry
            .query(&RegistryQuery {
                run_state_in: vec![RunState::Running, RunState::Waiting],
                ..Default::default()
            })
            .await?;
        let my_refs = me.map(declared_refs).unwrap_or_default();
        let mut out = Vec::new();
        for e in entries {
            if let Some(m) = me {
                if m.session_id == e.session_id {
                    continue;
                }
            }
            let refs = declared_refs(&e);
            let overlap: Vec<String> = refs
                .iter()
                .filter(|r| my_refs.iter().any(|m| refs_overlap(m, r)))
                .cloned()
                .collect();
            let relation = match me {
                Some(m) if same_target(m, &e) => Relation::SameTarget,
                _ if !overlap.is_empty() => Relation::Overlap,
                _ => Relation::Other,
            };
            // `status_only` sessions only expose their state summary.
            let activity = if e.agent_access == AgentAccess::StatusOnly {
                Activity {
                    summary: e.status.one_line_status.clone(),
                    touching: Vec::new(),
                    heartbeat_ms: e.status.activity.heartbeat_ms,
                }
            } else {
                e.status.activity.clone()
            };
            let possibly_interrupted = e.status.run_state == RunState::Running
                && now.saturating_sub(e.status.activity.heartbeat_ms.max(e.status.updated_at_ms))
                    > STALE_HEARTBEAT_MS;
            out.push(ActiveSession {
                session_id: e.session_id.clone(),
                kind: e.kind,
                run_state: e.status.run_state,
                objective: e.objective.chars().take(200).collect(),
                activity,
                relation,
                overlap,
                possibly_interrupted,
                driver: e.driver.principal.clone(),
            });
        }
        out.sort_by(|a, b| {
            a.relation
                .cmp(&b.relation)
                .then(b.overlap.len().cmp(&a.overlap.len()))
                .then(a.session_id.cmp(&b.session_id))
        });
        out.truncate(limit);
        Ok(out)
    }
}

/// Render the `<active_sessions>` block for the turn variable section.
pub fn render_active_sessions(list: &[ActiveSession]) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let mut s = String::from("<active_sessions>\n");
    for a in list {
        let mark = match a.relation {
            Relation::SameTarget => " relation=\"same_target\"",
            Relation::Overlap => " relation=\"overlap\"",
            Relation::Other => "",
        };
        let stale = if a.possibly_interrupted {
            " possibly_interrupted=\"true\""
        } else {
            ""
        };
        s.push_str(&format!(
            "<session id=\"{}\" state=\"{}\"{mark}{stale}>",
            a.session_id,
            a.run_state.as_str()
        ));
        let summary = if a.activity.summary.is_empty() {
            a.objective.clone()
        } else {
            a.activity.summary.clone()
        };
        s.push_str(&summary.replace('<', "&lt;"));
        for t in &a.activity.touching {
            s.push_str(&format!(
                "\n  <touching kind=\"{}\" mode=\"{}\">{}</touching>",
                t.kind, t.mode, t.target
            ));
        }
        if !a.overlap.is_empty() {
            s.push_str(&format!("\n  <overlap>{}</overlap>", a.overlap.join(", ")));
        }
        s.push_str("</session>\n");
    }
    s.push_str("</active_sessions>");
    Some(s)
}
