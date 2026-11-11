//! What the owner's pages read across the whole Agent (Agent UI upgrade
//! TODO): Memory without a Session identity, the files the Agent delivered,
//! and the summary the home page and the MessageHub panel share.
//!
//! - `memory.overview`: counts of both layers, the latest additions, when
//!   the consolidation Goal last settled something.
//! - `memory.list {layer, limit, cursor?}`: one layer newest first.
//! - `memory.read {reference}`: a cognition with where it comes from, or a
//!   perception with its record.
//! - `memory.forget {reference, expected_revision?, reason?}` (owner): a
//!   cognition gets a `deleted` revision, an unsettled perception is
//!   discarded and cleaned. `busy` while a consolidation runs.
//! - `deliveries.recent {limit, before?, route_key?}`: the delivery index.
//! - `home.summary`: sessions, Memory, goals, skills and runtimes in
//!   numbers.

use libopendan::memory::{Memory, MemoryConfig, MemoryError, MemoryLayer, MemoryResult, CONSOLIDATION_LEASE};
use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::OpenDanError;
use serde::Serialize;
use serde_json::{json, Value};

use crate::home::Home;

const DEFAULT_MEMORY_PAGE: usize = 20;
const DEFAULT_DELIVERY_PAGE: usize = 20;

/// The sessions of the home page's overview, by what the owner would call
/// them. One definition for the page and the panel:
///
/// - `working`: running or ready, a Turn open, or waiting for work of its
///   own (tasks, sub sessions, a Goal's next run).
/// - `waiting`: not finished and not working: only someone's input moves it.
/// - `failed`: finished as failed.
/// - `interrupted`: paused by a restart of its host, until someone resumes
///   it (counted in `working` or `waiting` as well).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SessionCounts {
    pub working: usize,
    pub waiting: usize,
    pub failed: usize,
    pub interrupted: usize,
    pub total: usize,
}

pub fn is_working(status: &SessionStatus) -> bool {
    status.run_state != RunState::Finished
        && status.paused.is_none()
        && (matches!(status.run_state, RunState::Running | RunState::Ready)
            || status.turn_open
            || (status.run_state == RunState::Waiting && !status.needs_input))
}

pub fn is_waiting(status: &SessionStatus) -> bool {
    status.run_state != RunState::Finished && !is_working(status)
}

pub fn is_failed(status: &SessionStatus) -> bool {
    status.run_state == RunState::Finished && status.outcome == Some(Outcome::Failed)
}

/// Paused because its host restarted in the middle of its work.
pub fn is_interrupted(status: &SessionStatus) -> bool {
    status.run_state != RunState::Finished && status.paused == Some(PauseSource::Interrupted)
}

pub fn count_sessions<'a>(statuses: impl IntoIterator<Item = &'a SessionStatus>) -> SessionCounts {
    let mut counts = SessionCounts::default();
    for status in statuses {
        counts.total += 1;
        counts.working += is_working(status) as usize;
        counts.waiting += is_waiting(status) as usize;
        counts.failed += is_failed(status) as usize;
        counts.interrupted += is_interrupted(status) as usize;
    }
    counts
}

fn agent_root(agent: &dyn AgentStateClient) -> Result<std::path::PathBuf, OpenDanError> {
    agent
        .agent_root()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| OpenDanError::InvalidArgument("the owner's views are served where the AgentRoot is".into()))
}

/// What the owner is told when Memory refuses: `busy` (try again), a
/// conflict (`memory_conflict: …`: the entry changed, read it again), or
/// the facade's own kinds.
fn memory_err(e: MemoryError) -> OpenDanError {
    match e {
        MemoryError::Conflict(m) => OpenDanError::Other(format!("memory_conflict: {m}")),
        MemoryError::LockTimeout(resource) => OpenDanError::Busy { resource: format!("memory:{resource}"), holder: None },
        other => other.into(),
    }
}

async fn with_memory<T: Send + 'static>(
    agent: &dyn AgentStateClient,
    f: impl FnOnce(Memory) -> MemoryResult<T> + Send + 'static,
) -> Result<T, OpenDanError> {
    let root = agent_root(agent)?;
    tokio::task::spawn_blocking(move || f(Memory::open(root, MemoryConfig::default())))
        .await
        .map_err(|e| OpenDanError::Other(e.to_string()))?
        .map_err(memory_err)
}

fn encode<T: Serialize>(value: &T) -> Result<Value, OpenDanError> {
    serde_json::to_value(value).map_err(|e| OpenDanError::Other(e.to_string()))
}

fn text(params: &Value, name: &str) -> Result<Option<String>, OpenDanError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty())),
        Some(_) => Err(OpenDanError::InvalidArgument(format!("param `{name}` is a string"))),
    }
}

fn number(params: &Value, name: &str) -> Result<Option<u64>, OpenDanError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| OpenDanError::InvalidArgument(format!("param `{name}` is a non-negative number"))),
    }
}

fn reference(params: &Value) -> Result<String, OpenDanError> {
    text(params, "reference")?.ok_or_else(|| OpenDanError::InvalidArgument("param `reference` is required".into()))
}

/// The consolidation Goal and when it last settled something. A Goal's id
/// is not fixed: it is found by its builtin mark.
async fn last_consolidation(agent: &dyn AgentStateClient) -> Result<(Option<String>, Option<u64>), OpenDanError> {
    let Some(goal) = agent.goals().list().await?.into_iter().find(|g| g.is_builtin(BUILTIN_MEMORY_MAINTENANCE)) else {
        return Ok((None, None));
    };
    let at = agent
        .goals()
        .state(&goal.goal_id)
        .await?
        .and_then(|s| s.maintenance)
        .and_then(|m| m.last_successful_consolidation_at_ms);
    Ok((Some(goal.goal_id), at))
}

impl Home {
    /// `memory.overview`.
    pub async fn memory_overview(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let overview = with_memory(agent, |m| m.owner_overview()).await?;
        let (goal_id, at) = last_consolidation(agent).await?;
        let mut out = encode(&overview)?;
        out["last_consolidation_at_ms"] = json!(at);
        out["maintenance_goal_id"] = json!(goal_id);
        Ok(out)
    }

    /// `memory.list {layer, limit?, cursor?}`.
    pub async fn memory_list(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let layer: MemoryLayer = serde_json::from_value(params.get("layer").cloned().unwrap_or(Value::Null))
            .map_err(|_| OpenDanError::InvalidArgument("param `layer` is cognition | perception".into()))?;
        let limit = number(params, "limit")?.map(|n| n as usize).unwrap_or(DEFAULT_MEMORY_PAGE);
        let cursor = text(params, "cursor")?;
        encode(&with_memory(agent, move |m| m.owner_list(layer, limit, cursor.as_deref())).await?)
    }

    /// `memory.read {reference}`.
    pub async fn memory_read(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let reference = reference(params)?;
        encode(&with_memory(agent, move |m| m.owner_read(&reference)).await?)
    }

    /// `memory.forget {reference, expected_revision?, reason?}`: the service
    /// takes the consolidation lease for the call, so it never runs beside a
    /// consolidation (`busy`: one is running, nothing changed). It is a
    /// commit of its own and does not count as a consolidation: the Goal's
    /// "last consolidation" does not move.
    pub async fn memory_forget(&self, agent: &dyn AgentStateClient, params: &Value, who: &str) -> Result<Value, OpenDanError> {
        let reference = reference(params)?;
        let expected = number(params, "expected_revision")?;
        let reason = text(params, "reason")?;
        let holder = HolderInfo {
            runner_id: libopendan::ids::new_runner_id(),
            principal: who.to_string(),
            host: Some(libopendan::runtime::native_host_id()),
            pid: std::process::id(),
            runtime_id: None,
        };
        let lease = match agent.locks().acquire(CONSOLIDATION_LEASE, holder)? {
            libopendan::lock::Acquire::Acquired(lease) => lease,
            libopendan::lock::Acquire::Busy(holder) => {
                return Err(OpenDanError::Busy {
                    resource: CONSOLIDATION_LEASE.to_string(),
                    holder: holder.and_then(|h| serde_json::to_value(h).ok()),
                })
            }
        };
        let who = who.to_string();
        let done = with_memory(agent, move |m| {
            let done = m.consolidator(&lease, "owner")?.forget(&reference, expected, reason.as_deref(), &who);
            lease.release();
            done
        })
        .await?;
        log::info!("memory: {} {} by the owner", done.reference, done.state);
        encode(&done)
    }

    /// `deliveries.recent {limit?, before?, route_key?}`.
    pub async fn deliveries_recent(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let root = agent_root(agent)?;
        let limit = number(params, "limit")?.map(|n| n as usize).unwrap_or(DEFAULT_DELIVERY_PAGE);
        let before = number(params, "before")?;
        let route_key = text(params, "route_key")?;
        let page = tokio::task::spawn_blocking(move || libopendan::deliveries::recent(&root, limit, before, route_key.as_deref()))
            .await
            .map_err(|e| OpenDanError::Other(e.to_string()))??;
        encode(&page)
    }

    /// `home.summary`.
    pub async fn summary(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let entries = agent.sessions().query(&RegistryQuery::default()).await?;
        let sessions = count_sessions(entries.iter().map(|e| &e.status));
        let memory = with_memory(agent, |m| m.owner_overview()).await?;
        let (_, consolidated_at) = last_consolidation(agent).await?;
        let goals = agent.goals().list().await?;
        let skills = agent.behaviors().skills(None).await?;
        let enabled = |kind: SkillKind| skills.iter().filter(|s| s.kind == kind && s.enabled).count();
        let runtimes: Vec<RuntimeRecord> = agent
            .runtimes()
            .list()
            .await?
            .into_iter()
            .filter(|r| r.lifecycle == RuntimeLifecycle::Active && r.enabled)
            .collect();
        Ok(json!({
            "sessions": sessions,
            "memory": {
                "perceptions": memory.perceptions.total,
                "pending": memory.perceptions.pending,
                "cognitions": memory.cognitions.current,
                "last_consolidation_at_ms": consolidated_at,
            },
            "goals": { "active": goals.iter().filter(|g| g.status == GoalStatus::Active).count() },
            "skills": {
                "work": enabled(SkillKind::Work),
                "plan": enabled(SkillKind::Plan),
                "goal": enabled(SkillKind::Goal),
                "disabled": skills.iter().filter(|s| !s.enabled).count(),
            },
            "runtimes": {
                "enabled": runtimes.len(),
                "unavailable": runtimes.iter().filter(|r| r.availability != RuntimeAvailability::Available).count(),
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(run_state: RunState, f: impl FnOnce(&mut SessionStatus)) -> SessionStatus {
        let mut s = SessionStatus { run_state, ..SessionStatus::created(0) };
        f(&mut s);
        s
    }

    /// The same cases as `isWorking` / `isWaiting` / `isFailed` of the home page had.
    #[test]
    fn sessions_are_counted_the_way_the_home_page_names_them() {
        let cases = [
            ("running", status(RunState::Running, |_| {}), (true, false, false)),
            ("ready", status(RunState::Ready, |_| {}), (true, false, false)),
            ("created", status(RunState::Created, |_| {}), (false, true, false)),
            ("an idle conversation", status(RunState::Waiting, |s| s.needs_input = true), (false, true, false)),
            ("waiting for its own work", status(RunState::Waiting, |_| {}), (true, false, false)),
            ("asked inside an open Turn", status(RunState::Waiting, |s| { s.needs_input = true; s.turn_open = true }), (true, false, false)),
            ("done", status(RunState::Finished, |s| s.outcome = Some(Outcome::Succeeded)), (false, false, false)),
            ("stopped", status(RunState::Finished, |s| s.outcome = Some(Outcome::Stopped)), (false, false, false)),
            ("failed", status(RunState::Finished, |s| s.outcome = Some(Outcome::Failed)), (false, false, true)),
        ];
        for (name, s, expected) in &cases {
            assert_eq!((is_working(s), is_waiting(s), is_failed(s)), *expected, "{name}");
        }
        let counts = count_sessions(cases.iter().map(|(_, s, _)| s));
        assert_eq!(counts, SessionCounts { working: 4, waiting: 2, failed: 1, interrupted: 0, total: 9 });
    }

    const AGENT: &str = "did:bns:agentx";
    const OWNER: &str = "did:bns:bob";

    fn holder(name: &str) -> HolderInfo {
        HolderInfo { runner_id: name.into(), principal: name.into(), host: None, pid: std::process::id(), runtime_id: None }
    }

    fn note(root: &std::path::Path, sid: &str, words: &str) {
        use libopendan::memory::{ActorKind, Caller, PerceptionInput, SourceRef, SourceType};
        let lease = libopendan::lock::Lease::acquire(&format!("session:{sid}"), &root.join("leases").join(sid), holder(sid))
            .unwrap()
            .into_result("session")
            .unwrap();
        let mut source = SourceRef::event(SourceType::SessionEvent, &format!("{sid}/{words}"));
        source.actor_kind = Some(ActorKind::User);
        Memory::open(root, MemoryConfig::default())
            .record_perception(
                &Caller::new(sid, &[OWNER, AGENT], &[OWNER]),
                &lease,
                PerceptionInput { content: words.into(), source: Some(source), ..Default::default() },
            )
            .unwrap();
    }

    #[tokio::test]
    async fn the_owner_reads_and_forgets_memory_without_a_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("agent");
        let agent = libopendan::FsAgentStateClient::open(&root, AGENT, None, None).unwrap();
        let home = Home::new(std::sync::Arc::new(crate::records::LocalRecords::memory()), Some(OWNER.into()), None);

        let empty = home.memory_overview(&agent).await.unwrap();
        assert_eq!(empty["cognitions"]["current"], 0);
        assert_eq!((empty["perceptions"]["total"].as_u64(), empty["perceptions"]["pending"].as_u64()), (Some(0), Some(0)));
        assert!(empty["last_consolidation_at_ms"].is_null() && empty["maintenance_goal_id"].is_null(), "{empty}");
        assert_eq!(empty["recent"], json!({ "cognitions": [], "perceptions": [] }));

        note(&root, "ui-a", "likes tea");
        note(&root, "ui-a", "lives in Hilo");
        let overview = home.memory_overview(&agent).await.unwrap();
        assert_eq!((overview["perceptions"]["total"].as_u64(), overview["perceptions"]["pending"].as_u64()), (Some(2), Some(2)));
        let page = home.memory_list(&agent, &json!({ "layer": "perception", "limit": 1 })).await.unwrap();
        assert_eq!(page["entries"][0]["reference"], "ui-a:2");
        assert_eq!((page["entries"][0]["layer"].as_str(), page["entries"][0]["deletable"].as_bool()), (Some("perception"), Some(true)));
        let next = home.memory_list(&agent, &json!({ "layer": "perception", "limit": 1, "cursor": page["next_cursor"] })).await.unwrap();
        assert_eq!(next["entries"][0]["reference"], "ui-a:1");
        assert!(next.get("next_cursor").is_none(), "{next}");
        assert_eq!(home.memory_list(&agent, &json!({ "layer": "cognition" })).await.unwrap(), json!({ "entries": [] }));
        assert!(matches!(home.memory_list(&agent, &json!({ "layer": "all" })).await, Err(OpenDanError::InvalidArgument(_))));

        let read = home.memory_read(&agent, &json!({ "reference": "ui-a:1" })).await.unwrap();
        assert_eq!((read["layer"].as_str(), read["perception"]["record"]["summary"].as_str()), (Some("perception"), Some("likes tea")));
        assert!(matches!(home.memory_read(&agent, &json!({ "reference": "ui-a:7" })).await, Err(OpenDanError::NotFound(_))));

        // While a consolidation holds the lease the owner is told to come back; nothing changes.
        let running = agent.locks().acquire(CONSOLIDATION_LEASE, holder("goal")).unwrap().into_result(CONSOLIDATION_LEASE).unwrap();
        let busy = home.memory_forget(&agent, &json!({ "reference": "ui-a:1" }), OWNER).await;
        assert!(matches!(busy, Err(OpenDanError::Busy { .. })), "{busy:?}");
        assert_eq!(home.memory_overview(&agent).await.unwrap()["perceptions"]["total"], 2);
        running.release();

        let done = home.memory_forget(&agent, &json!({ "reference": "ui-a:1", "reason": "wrong" }), OWNER).await.unwrap();
        assert_eq!((done["layer"].as_str(), done["state"].as_str()), (Some("perception"), Some("discarded")));
        let overview = home.memory_overview(&agent).await.unwrap();
        assert_eq!((overview["perceptions"]["total"].as_u64(), overview["perceptions"]["pending"].as_u64()), (Some(1), Some(1)));
        let again = home.memory_forget(&agent, &json!({ "reference": "ui-a:1" }), OWNER).await.unwrap_err();
        assert!(again.to_string().starts_with("memory_conflict:"), "{again}");
        // The lease is free again for the consolidation.
        agent.locks().acquire(CONSOLIDATION_LEASE, holder("goal")).unwrap().into_result(CONSOLIDATION_LEASE).unwrap().release();

        let summary = home.summary(&agent).await.unwrap();
        assert_eq!(summary["memory"], json!({ "perceptions": 1, "pending": 1, "cognitions": 0, "last_consolidation_at_ms": null }));
        assert_eq!(summary["sessions"], json!({ "working": 0, "waiting": 0, "failed": 0, "interrupted": 0, "total": 0 }));
        for key in ["goals", "skills", "runtimes"] {
            assert!(summary[key].is_object(), "{summary}");
        }

        assert_eq!(home.deliveries_recent(&agent, &json!({})).await.unwrap(), json!({ "entries": [] }));
        libopendan::deliveries::record(
            &root,
            &libopendan::deliveries::DeliveryRecord {
                at_ms: 1,
                name: "a.png".into(),
                obj_id: "cyfile:x".into(),
                url: None,
                mime: None,
                size: None,
                session_id: "work-1".into(),
                ui_session_id: Some("ui-a".into()),
                route_key: Some("route-a".into()),
                source: libopendan::deliveries::DeliverySource::Report,
            },
        )
        .unwrap();
        let page = home.deliveries_recent(&agent, &json!({ "limit": 5, "route_key": "route-a" })).await.unwrap();
        assert_eq!((page["entries"][0]["name"].as_str(), page["entries"][0]["source"].as_str()), (Some("a.png"), Some("report")));
        assert_eq!(home.deliveries_recent(&agent, &json!({ "route_key": "route-b" })).await.unwrap(), json!({ "entries": [] }));
    }
}
