//! Goals in the Agent State (`state/goals/<goal_id>/`, see
//! [`crate::protocol::goal`]).
//!
//! Writers: the Goal definition (`goal.json`, `revisions/`, `changes.jsonl`)
//! is changed only through [`GoalStore`] under the goal's lock, content
//! changes compare the revision the caller saw (CAS). The Goal State and
//! the Activation records are written by the driver of the epoch's Goal
//! Session; `schedule.json` and the epoch archive by the host. Those writes
//! need the AgentRoot and are never served over kRPC.
//!
//! Permissions are decided by the caller's kind only (先能力后安全): the
//! owner side may do everything; a UI Session creates goals and steers
//! them; a Goal Session reads, archives itself or marks itself achieved;
//! a Work Session reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::registry::FsRegistry;
use super::SessionRegistry;

/// Who asks. Permissions follow the kind of `session` (the session the
/// call is made from, self-reported like a poster's `from`); without one
/// the caller is the owner side.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCaller {
    pub principal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl GoalCaller {
    pub fn owner(principal: &str) -> Self {
        Self {
            principal: principal.to_string(),
            session: None,
        }
    }

    pub fn from_session(principal: &str, sid: &str) -> Self {
        Self {
            principal: principal.to_string(),
            session: Some(sid.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalCreate {
    /// Idempotency key of the creator: the same key returns the same Goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Default: the first line of the objective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub objective: String,
    #[serde(default)]
    pub scope: GoalScope,
    #[serde(default)]
    pub lifecycle: GoalLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub achieved_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthy_state: Option<String>,
    #[serde(default)]
    pub distance_evaluation: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub initial_skills: Vec<GoalSkill>,
    #[serde(default)]
    pub beneficiaries: Vec<String>,
    #[serde(default)]
    pub budget: GoalBudget,
    #[serde(default)]
    pub schedule: GoalScheduleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// Default: the caller's principal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Marks of later phases. The `opendan` key is the host's (seeding):
    /// only the owner side without a session may set it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    /// Source (TypeScript) of the check script the Goal starts with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_script: Option<String>,
    /// The package script it is a copy of (owner side only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_package: Option<String>,
}

/// A change of a Goal's check: its script and its timetable. The Goal
/// Session may make it for its own Goal; it is recorded (`changes.jsonl`
/// kind `check`) but is not a content revision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalCheckChange {
    /// New script source (TypeScript).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// The script is a copy of this package script (owner side only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Go back to the package script the Goal started from.
    #[serde(default)]
    pub restore_package: bool,
    /// Remove the script (every scheduled check passes).
    #[serde(default)]
    pub clear_script: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `sha256` of a check script's source.
pub fn script_digest(script: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(script.as_bytes()))
}

/// Where version `version` of a Goal's check script is on an AgentRoot.
pub fn check_script_path(agent_root: &Path, goal_id: &str, version: u64) -> PathBuf {
    super::fs_client::AgentLayout::new(agent_root)
        .goals_dir()
        .join(goal_id)
        .join("checks")
        .join(format!("{version}.ts"))
}

fn check_schedule(s: &GoalScheduleSpec) -> Result<()> {
    if s.every_secs == Some(0) {
        return Err(OpenDanError::InvalidArgument("schedule.every_secs must be > 0".into()));
    }
    crate::goal_time::validate_timetable(s.every_secs, s.cron.as_deref(), s.timezone.as_deref())
        .map_err(|e| OpenDanError::InvalidArgument(format!("schedule: {e}")))
}

/// A content revision: fields left out stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRevise {
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<GoalScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<GoalLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub achieved_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthy_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_evaluation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_skills: Option<Vec<GoalSkill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beneficiaries: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<GoalBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<GoalScheduleSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// Replaces the extensions (the `opendan` key: owner side only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalStatusChange {
    pub status: GoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `achieved`: what shows it (required from the Goal Session itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalReopen {
    pub reason: String,
    /// Keep only the facts of the Goal State (default: all of it).
    #[serde(default)]
    pub fresh_state: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRestart {
    pub reason: String,
    /// Keep the whole Goal State (default: only the facts).
    #[serde(default)]
    pub carry_state: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// The newest `limit` records (oldest first); 0 = all.
    #[serde(default)]
    pub limit: usize,
}

/// The Goals of an agent (Agent State facade).
#[async_trait]
pub trait GoalStore: Send + Sync {
    async fn list(&self) -> Result<Vec<GoalDefinition>>;
    async fn get(&self, goal_id: &str) -> Result<Option<GoalDefinition>>;
    /// The immutable snapshot of a content revision.
    async fn revision(&self, goal_id: &str, revision: u64) -> Result<Option<GoalDefinition>>;
    /// The newest `limit` changes (oldest first; 0 = all).
    async fn changes(&self, goal_id: &str, limit: usize) -> Result<Vec<GoalChange>>;
    async fn create(&self, req: &GoalCreate, caller: &GoalCaller) -> Result<GoalDefinition>;
    async fn revise(&self, goal_id: &str, req: &GoalRevise, caller: &GoalCaller)
        -> Result<GoalDefinition>;
    /// pause / resume (`active`) / archive / achieved.
    async fn set_status(
        &self,
        goal_id: &str,
        req: &GoalStatusChange,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition>;
    /// achieved / archived → active in a new epoch.
    async fn reopen(&self, goal_id: &str, req: &GoalReopen, caller: &GoalCaller)
        -> Result<GoalDefinition>;
    /// A new epoch with a fresh context (active / paused stay as they are).
    async fn restart(&self, goal_id: &str, req: &GoalRestart, caller: &GoalCaller)
        -> Result<GoalDefinition>;
    /// Its check script and timetable (the Goal Session may change its own).
    async fn set_check(&self, goal_id: &str, req: &GoalCheckChange, caller: &GoalCaller)
        -> Result<GoalDefinition>;
    /// Source of a version of the check script (`None`: the current one).
    async fn check_script(&self, goal_id: &str, version: Option<u64>) -> Result<Option<String>>;
    async fn state(&self, goal_id: &str) -> Result<Option<GoalState>>;
    /// The Goal State an ended epoch left.
    async fn epoch_state(&self, goal_id: &str, epoch: u64) -> Result<Option<GoalState>>;
    async fn activations(&self, goal_id: &str, q: &ActivationQuery) -> Result<Vec<GoalActivation>>;
    async fn schedule(&self, goal_id: &str) -> Result<Option<GoalSchedule>>;

    // ---- the driver of the epoch's Goal Session (under its lease) ----
    /// Publish the Goal State the session committed (ignored when the file
    /// already has this or a newer rev; refused for an older epoch).
    async fn write_state(&self, lease: &Lease, state: &GoalState) -> Result<()>;
    /// Append Activation records; idempotent by `(epoch, seq)`.
    async fn append_activations(&self, lease: &Lease, records: &[GoalActivation]) -> Result<()>;

    // ---- the host ----
    /// Make `state.json` the state epoch `epoch` starts with: the previous
    /// epoch's is archived under `epochs/<n>/` and carried over as the
    /// epoch says (idempotent).
    async fn prepare_epoch(&self, goal_id: &str, epoch: u64) -> Result<GoalState>;
    /// The epoch's Goal Session finished.
    async fn end_epoch(
        &self,
        goal_id: &str,
        epoch: u64,
        end: EpochEnd,
        reason: Option<String>,
    ) -> Result<GoalDefinition>;
    async fn write_schedule(&self, schedule: &GoalSchedule) -> Result<()>;
}

/// What the caller is, for the rules of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerKind {
    Owner,
    Ui(String),
    /// A Goal Session: its sid and its goal.
    Goal(String, String),
    Work(String),
    Other(String),
}

impl CallerKind {
    /// `changes.jsonl` `via`.
    pub fn via(&self) -> String {
        match self {
            CallerKind::Owner => "owner".into(),
            CallerKind::Ui(sid) => format!("ui_session:{sid}"),
            CallerKind::Goal(sid, _) => format!("goal_session:{sid}"),
            CallerKind::Work(sid) => format!("work_session:{sid}"),
            CallerKind::Other(sid) => format!("session:{sid}"),
        }
    }
}

fn forbidden(what: &str, kind: &CallerKind) -> OpenDanError {
    OpenDanError::Forbidden(format!("{} may not {what}", kind.via()))
}

/// The `opendan` extension is the host's: readable by this build and set by
/// the owner side only.
fn check_extensions(ext: &BTreeMap<String, Value>, ck: &CallerKind) -> Result<()> {
    let Some(v) = ext.get(OPENDAN_EXTENSION) else {
        return Ok(());
    };
    if *ck != CallerKind::Owner {
        return Err(forbidden(
            &format!("set the `{OPENDAN_EXTENSION}` extension of a goal (the host seeds it)"),
            ck,
        ));
    }
    serde_json::from_value::<OpendanGoalExtension>(v.clone()).map_err(|e| {
        OpenDanError::InvalidArgument(format!("extensions.{OPENDAN_EXTENSION}: {e}"))
    })?;
    Ok(())
}

fn check_goal_id(goal_id: &str) -> Result<()> {
    crate::ids::validate_session_id(goal_id)
        .map_err(|_| OpenDanError::InvalidArgument(format!("bad goal id `{goal_id}`")))
}

fn first_line(text: &str, max: usize) -> String {
    text.trim()
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(max)
        .collect()
}

/// Goals on the AgentRoot files.
pub struct FsGoals {
    layout: AgentLayout,
    agent_did: String,
    registry: FsRegistry,
}

/// The exclusive lock of one goal (held while its files change).
struct GoalLock(std::fs::File);

impl Drop for GoalLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

impl FsGoals {
    pub(super) fn new(layout: AgentLayout, agent_did: &str) -> Self {
        Self {
            registry: FsRegistry::new(layout.clone(), None, None),
            layout,
            agent_did: agent_did.to_string(),
        }
    }

    fn dir(&self, goal_id: &str) -> PathBuf {
        self.layout.goals_dir().join(goal_id)
    }

    fn goal_file(&self, goal_id: &str) -> PathBuf {
        self.dir(goal_id).join("goal.json")
    }

    fn lock(&self, goal_id: &str) -> Result<GoalLock> {
        let dir = self.dir(goal_id);
        std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        let path = dir.join(".lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| OpenDanError::io(&path, e))?;
        f.lock_exclusive().map_err(|e| OpenDanError::io(&path, e))?;
        Ok(GoalLock(f))
    }

    fn read(&self, goal_id: &str) -> Result<Option<GoalDefinition>> {
        check_goal_id(goal_id)?;
        fsutil::read_json_opt(&self.goal_file(goal_id))
    }

    fn must(&self, goal_id: &str) -> Result<GoalDefinition> {
        self.read(goal_id)?
            .ok_or_else(|| OpenDanError::NotFound(format!("goal {goal_id}")))
    }

    fn append_change(&self, goal: &mut GoalDefinition, mut change: GoalChange) -> Result<()> {
        goal.change_seq += 1;
        change.seq = goal.change_seq;
        let line = fsutil::to_json_lines(&[change])?;
        fsutil::append_batch(&self.dir(&goal.goal_id).join("changes.jsonl"), &line)?;
        Ok(())
    }

    fn write_goal(&self, goal: &GoalDefinition) -> Result<()> {
        fsutil::atomic_replace_json(&self.goal_file(&goal.goal_id), goal)
    }

    fn write_revision(&self, goal: &GoalDefinition) -> Result<()> {
        let path = self
            .dir(&goal.goal_id)
            .join("revisions")
            .join(format!("{}.json", goal.revision));
        // Immutable: a redo after a crash finds it written.
        fsutil::publish_noreplace_json(&path, goal).map(|_| ())
    }

    async fn caller_kind(&self, caller: &GoalCaller) -> Result<CallerKind> {
        let Some(sid) = &caller.session else {
            return Ok(CallerKind::Owner);
        };
        let e = self
            .registry
            .lookup(sid)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("calling session {sid}")))?;
        Ok(match e.kind {
            SessionKind::Ui => CallerKind::Ui(sid.clone()),
            SessionKind::Goal => CallerKind::Goal(sid.clone(), e.goal_id.unwrap_or_default()),
            SessionKind::Work => CallerKind::Work(sid.clone()),
        })
    }

    /// Work of the goal that is not finished.
    async fn unsettled_works(&self, goal_id: &str) -> Result<Vec<String>> {
        Ok(self
            .registry
            .query(&RegistryQuery {
                goal_id: Some(goal_id.to_string()),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|e| e.kind != SessionKind::Goal && e.status.run_state != RunState::Finished)
            .map(|e| e.session_id)
            .collect())
    }

    fn change(kind: &str, caller: &GoalCaller, ck: &CallerKind, reason: Option<String>) -> GoalChange {
        GoalChange {
            seq: 0,
            at_ms: crate::now_ms(),
            kind: kind.to_string(),
            who: caller.principal.clone(),
            via: ck.via(),
            reason,
            revision: None,
            epoch: None,
            detail: Value::Null,
        }
    }

    fn new_epoch(
        &self,
        goal: &mut GoalDefinition,
        start: EpochStart,
        reason: &str,
        keep: StateKeep,
        now: u64,
    ) -> Result<()> {
        let epoch = goal.session.epoch + 1;
        let sid = super::registry::reserve_session_id(
            &self.layout,
            SessionKind::Goal,
            &crate::ids::goal_session_key(&self.agent_did, &goal.goal_id, epoch),
            None,
        )?;
        goal.epochs.push(GoalEpoch {
            epoch,
            sid: sid.clone(),
            started_at_ms: now,
            start,
            reason: Some(reason.to_string()),
            keep,
            ended_at_ms: None,
            end: None,
            end_reason: None,
        });
        goal.session = GoalSessionRef { epoch, sid };
        Ok(())
    }

    fn state_file(&self, goal_id: &str) -> PathBuf {
        self.dir(goal_id).join("state.json")
    }

    /// Write a new version of a Goal's check script (immutable).
    fn store_script(
        &self,
        goal_id: &str,
        script: &str,
        source: String,
        package: Option<String>,
        now: u64,
    ) -> Result<GoalCheckScript> {
        if script.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("the check script is empty".into()));
        }
        if script.len() > crate::goal_check::CHECK_SCRIPT_MAX {
            return Err(OpenDanError::InvalidArgument(format!(
                "the check script is {} bytes, more than {}",
                script.len(),
                crate::goal_check::CHECK_SCRIPT_MAX
            )));
        }
        let dir = self.dir(goal_id).join("checks");
        std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        let latest = std::fs::read_dir(&dir)
            .map_err(|e| OpenDanError::io(&dir, e))?
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".ts")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let version = latest + 1;
        let path = dir.join(format!("{version}.ts"));
        fsutil::atomic_replace(&path, script.as_bytes())?;
        Ok(GoalCheckScript {
            version,
            sha256: script_digest(script),
            source,
            package,
            updated_at_ms: now,
        })
    }
}

fn last_lines<T: serde::de::DeserializeOwned>(path: &Path, limit: usize) -> Result<Vec<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OpenDanError::io(path, e)),
    };
    let mut out: Vec<T> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if limit > 0 && out.len() > limit {
        out.drain(..out.len() - limit);
    }
    Ok(out)
}

#[async_trait]
impl GoalStore for FsGoals {
    async fn list(&self) -> Result<Vec<GoalDefinition>> {
        let dir = self.layout.goals_dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || crate::ids::validate_session_id(&name).is_err() {
                continue;
            }
            match fsutil::read_json_opt::<GoalDefinition>(&e.path().join("goal.json")) {
                Ok(Some(g)) => out.push(g),
                Ok(None) => {}
                Err(err) => log::warn!("goal {name}: {err}"),
            }
        }
        out.sort_by(|a, b| (a.created_at_ms, &a.goal_id).cmp(&(b.created_at_ms, &b.goal_id)));
        Ok(out)
    }

    async fn get(&self, goal_id: &str) -> Result<Option<GoalDefinition>> {
        self.read(goal_id)
    }

    async fn revision(&self, goal_id: &str, revision: u64) -> Result<Option<GoalDefinition>> {
        check_goal_id(goal_id)?;
        fsutil::read_json_opt(
            &self
                .dir(goal_id)
                .join("revisions")
                .join(format!("{revision}.json")),
        )
    }

    async fn changes(&self, goal_id: &str, limit: usize) -> Result<Vec<GoalChange>> {
        check_goal_id(goal_id)?;
        last_lines(&self.dir(goal_id).join("changes.jsonl"), limit)
    }

    async fn create(&self, req: &GoalCreate, caller: &GoalCaller) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        if !matches!(ck, CallerKind::Owner | CallerKind::Ui(_)) {
            return Err(forbidden("create a goal", &ck));
        }
        if req.objective.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("a goal needs an objective".into()));
        }
        check_schedule(&req.schedule)?;
        if req.schedule.check.is_some() {
            return Err(OpenDanError::InvalidArgument(
                "schedule.check is set from check_script, not given".into(),
            ));
        }
        if req.check_package.is_some() && ck != CallerKind::Owner {
            return Err(forbidden("name a package check script", &ck));
        }
        if req.budget.period.window_secs == Some(0) {
            return Err(OpenDanError::InvalidArgument("budget.period.window_secs must be > 0".into()));
        }
        check_extensions(&req.extensions, &ck)?;
        let goal_id =
            crate::ids::new_goal_id(&self.agent_did, &caller.principal, req.key.as_deref());
        let _lock = self.lock(&goal_id)?;
        if let Some(existing) = self.read(&goal_id)? {
            // The same key again: the goal it created.
            return Ok(existing);
        }
        let now = crate::now_ms();
        let sid = super::registry::reserve_session_id(
            &self.layout,
            SessionKind::Goal,
            &crate::ids::goal_session_key(&self.agent_did, &goal_id, 1),
            None,
        )?;
        let via = match &ck {
            CallerKind::Ui(sid) => format!("ui_session:{sid}"),
            _ => "owner".to_string(),
        };
        let mut goal = GoalDefinition {
            schema: GOAL_SCHEMA.to_string(),
            goal_id: goal_id.clone(),
            revision: 1,
            change_seq: 0,
            title: req
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| first_line(&req.objective, 80)),
            objective: req.objective.clone(),
            scope: req.scope.clone(),
            lifecycle: req.lifecycle,
            achieved_when: req.achieved_when.clone(),
            healthy_state: req.healthy_state.clone(),
            distance_evaluation: req.distance_evaluation.clone(),
            constraints: req.constraints.clone(),
            initial_skills: req.initial_skills.clone(),
            owner: req.owner.clone().unwrap_or_else(|| caller.principal.clone()),
            created_by: GoalCreatedBy {
                principal: caller.principal.clone(),
                via,
            },
            beneficiaries: req.beneficiaries.clone(),
            budget: req.budget.clone(),
            schedule: req.schedule.clone(),
            behavior: req.behavior.clone(),
            status: GoalStatus::Active,
            status_changed_at_ms: now,
            status_reason: None,
            status_by: Some(caller.principal.clone()),
            session: GoalSessionRef {
                epoch: 1,
                sid: sid.clone(),
            },
            epochs: vec![GoalEpoch {
                epoch: 1,
                sid,
                started_at_ms: now,
                start: EpochStart::Created,
                reason: None,
                keep: StateKeep::All,
                ended_at_ms: None,
                end: None,
                end_reason: None,
            }],
            idempotency_key: req.key.clone(),
            created_at_ms: now,
            updated_at_ms: now,
            extensions: req.extensions.clone(),
        };
        if let Some(script) = &req.check_script {
            let source = match &req.check_package {
                Some(p) => format!("package:{p}"),
                None => ck.via(),
            };
            goal.schedule.check = Some(self.store_script(&goal_id, script, source, req.check_package.clone(), now)?);
        }
        self.write_revision(&goal)?;
        let mut change = Self::change("created", caller, &ck, None);
        change.revision = Some(1);
        change.epoch = Some(1);
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn revise(
        &self,
        goal_id: &str,
        req: &GoalRevise,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        match &ck {
            CallerKind::Owner => {}
            CallerKind::Ui(sid) if goal.created_by.via == format!("ui_session:{sid}") => {}
            CallerKind::Ui(_) => return Err(forbidden("revise a goal another session created", &ck)),
            _ => return Err(forbidden("revise a goal's definition, budget or schedule", &ck)),
        }
        if goal.revision != req.expected_revision {
            return Err(OpenDanError::GoalConflict(format!(
                "goal {goal_id} is at revision {}, not {}",
                goal.revision, req.expected_revision
            )));
        }
        let before = goal.clone();
        macro_rules! take {
            ($f:ident) => {
                if let Some(v) = &req.$f {
                    goal.$f = v.clone();
                }
            };
        }
        take!(title);
        take!(objective);
        take!(scope);
        take!(lifecycle);
        take!(distance_evaluation);
        take!(constraints);
        take!(initial_skills);
        take!(beneficiaries);
        take!(budget);
        if let Some(v) = &req.schedule {
            // The check script changes through `set_check` only.
            let check = goal.schedule.check.clone();
            goal.schedule = v.clone();
            goal.schedule.check = check;
        }
        if req.achieved_when.is_some() {
            goal.achieved_when = req.achieved_when.clone();
        }
        if req.healthy_state.is_some() {
            goal.healthy_state = req.healthy_state.clone();
        }
        if req.behavior.is_some() {
            goal.behavior = req.behavior.clone();
        }
        if let Some(ext) = &req.extensions {
            if ext.get(OPENDAN_EXTENSION) != before.extensions.get(OPENDAN_EXTENSION) {
                check_extensions(ext, &ck)?;
                if ck != CallerKind::Owner {
                    return Err(forbidden(
                        &format!("change the `{OPENDAN_EXTENSION}` extension of a goal"),
                        &ck,
                    ));
                }
            }
            goal.extensions = ext.clone();
        }
        if goal.objective.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("a goal needs an objective".into()));
        }
        check_schedule(&goal.schedule)?;
        let mut cmp = goal.clone();
        cmp.revision = before.revision;
        if cmp == before {
            return Ok(before);
        }
        goal.revision += 1;
        goal.updated_at_ms = crate::now_ms();
        let changed: Vec<&str> = [
            ("title", before.title != goal.title),
            ("objective", before.objective != goal.objective),
            ("scope", before.scope != goal.scope),
            ("lifecycle", before.lifecycle != goal.lifecycle),
            ("achieved_when", before.achieved_when != goal.achieved_when),
            ("healthy_state", before.healthy_state != goal.healthy_state),
            ("distance_evaluation", before.distance_evaluation != goal.distance_evaluation),
            ("constraints", before.constraints != goal.constraints),
            ("initial_skills", before.initial_skills != goal.initial_skills),
            ("beneficiaries", before.beneficiaries != goal.beneficiaries),
            ("budget", before.budget != goal.budget),
            ("schedule", before.schedule != goal.schedule),
            ("behavior", before.behavior != goal.behavior),
            ("extensions", before.extensions != goal.extensions),
        ]
        .into_iter()
        .filter(|(_, c)| *c)
        .map(|(n, _)| n)
        .collect();
        self.write_revision(&goal)?;
        let mut change = Self::change("revised", caller, &ck, req.reason.clone());
        change.revision = Some(goal.revision);
        change.detail = json!({ "fields": changed });
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn set_status(
        &self,
        goal_id: &str,
        req: &GoalStatusChange,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        let from = goal.status;
        let to = req.status;
        match &ck {
            CallerKind::Owner | CallerKind::Ui(_) => {}
            CallerKind::Goal(_, g) if g == goal_id => match to {
                GoalStatus::Archived if req.reason.as_deref().is_some_and(|r| !r.trim().is_empty()) => {}
                GoalStatus::Archived => {
                    return Err(OpenDanError::InvalidArgument(
                        "a goal archiving itself gives the reason".into(),
                    ))
                }
                GoalStatus::Achieved => {
                    if goal.lifecycle != GoalLifecycle::Achieve {
                        return Err(forbidden("mark a maintain goal achieved", &ck));
                    }
                    if req.evidence.as_deref().is_none_or(|e| e.trim().is_empty()) {
                        return Err(OpenDanError::InvalidArgument(
                            "a goal marking itself achieved gives the evidence".into(),
                        ));
                    }
                }
                _ => return Err(forbidden("pause or resume its goal", &ck)),
            },
            _ => return Err(forbidden(&format!("set the status of goal {goal_id}"), &ck)),
        }
        if from == to {
            return Ok(goal);
        }
        let valid = match to {
            GoalStatus::Active => from == GoalStatus::Paused,
            GoalStatus::Paused => from == GoalStatus::Active,
            GoalStatus::Archived | GoalStatus::Achieved => from.is_live(),
        };
        if !valid {
            return Err(OpenDanError::GoalConflict(format!(
                "goal {goal_id} is {}; it cannot become {} (a finished goal is reopened)",
                from.as_str(),
                to.as_str()
            )));
        }
        if to == GoalStatus::Achieved {
            let open = self.unsettled_works(goal_id).await?;
            if !open.is_empty() {
                return Err(OpenDanError::GoalConflict(format!(
                    "goal {goal_id} still has unfinished work: {}",
                    open.join(", ")
                )));
            }
        }
        let now = crate::now_ms();
        goal.status = to;
        goal.status_changed_at_ms = now;
        goal.status_reason = req.reason.clone();
        goal.status_by = Some(caller.principal.clone());
        goal.updated_at_ms = now;
        let mut change = Self::change("status", caller, &ck, req.reason.clone());
        change.epoch = Some(goal.session.epoch);
        change.detail = json!({ "from": from, "to": to, "evidence": req.evidence });
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn reopen(
        &self,
        goal_id: &str,
        req: &GoalReopen,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        if !matches!(ck, CallerKind::Owner | CallerKind::Ui(_)) {
            return Err(forbidden("reopen a goal", &ck));
        }
        if req.reason.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("reopen needs a reason".into()));
        }
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        if goal.status.is_live() {
            return Err(OpenDanError::GoalConflict(format!(
                "goal {goal_id} is {}; only an achieved or archived goal is reopened (restart gives a live goal a new context)",
                goal.status.as_str()
            )));
        }
        let now = crate::now_ms();
        let from = goal.status;
        let keep = if req.fresh_state {
            StateKeep::Facts
        } else {
            StateKeep::All
        };
        self.new_epoch(&mut goal, EpochStart::Reopened, &req.reason, keep, now)?;
        goal.status = GoalStatus::Active;
        goal.status_changed_at_ms = now;
        goal.status_reason = Some(req.reason.clone());
        goal.status_by = Some(caller.principal.clone());
        goal.updated_at_ms = now;
        let mut change = Self::change("reopened", caller, &ck, Some(req.reason.clone()));
        change.epoch = Some(goal.session.epoch);
        change.detail = json!({ "from": from, "keep": keep });
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn restart(
        &self,
        goal_id: &str,
        req: &GoalRestart,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        if !matches!(ck, CallerKind::Owner | CallerKind::Ui(_)) {
            return Err(forbidden("restart a goal", &ck));
        }
        if req.reason.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("restart needs a reason".into()));
        }
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        if !goal.status.is_live() {
            return Err(OpenDanError::GoalConflict(format!(
                "goal {goal_id} is {}; reopen it instead",
                goal.status.as_str()
            )));
        }
        let now = crate::now_ms();
        let keep = if req.carry_state {
            StateKeep::All
        } else {
            StateKeep::Facts
        };
        let prev = goal.session.epoch;
        if let Some(e) = goal.epochs.iter_mut().find(|e| e.epoch == prev) {
            // Ends when its session finished (`end_epoch` stamps the time).
            e.end = Some(EpochEnd::Restarted);
            e.end_reason = Some(req.reason.clone());
        }
        self.new_epoch(&mut goal, EpochStart::Restarted, &req.reason, keep, now)?;
        goal.updated_at_ms = now;
        let mut change = Self::change("restarted", caller, &ck, Some(req.reason.clone()));
        change.epoch = Some(goal.session.epoch);
        change.detail = json!({ "previous_epoch": prev, "keep": keep });
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn set_check(
        &self,
        goal_id: &str,
        req: &GoalCheckChange,
        caller: &GoalCaller,
    ) -> Result<GoalDefinition> {
        let ck = self.caller_kind(caller).await?;
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        match &ck {
            CallerKind::Owner => {}
            CallerKind::Ui(sid) if goal.created_by.via == format!("ui_session:{sid}") => {}
            CallerKind::Goal(_, g) if g == goal_id => {}
            _ => return Err(forbidden(&format!("change the check of goal {goal_id}"), &ck)),
        }
        if req.package.is_some() && ck != CallerKind::Owner {
            return Err(forbidden("name a package check script", &ck));
        }
        let given = [req.script.is_some(), req.restore_package, req.clear_script].iter().filter(|x| **x).count();
        if given > 1 {
            return Err(OpenDanError::InvalidArgument(
                "give one of script, restore_package and clear_script".into(),
            ));
        }
        let before = goal.schedule.clone();
        if req.every_secs.is_some() && req.cron.is_some() {
            return Err(OpenDanError::InvalidArgument("every_secs or cron, not both".into()));
        }
        if let Some(e) = req.every_secs {
            goal.schedule.every_secs = Some(e);
            goal.schedule.cron = None;
        }
        if let Some(c) = &req.cron {
            goal.schedule.cron = Some(c.trim().to_string());
            goal.schedule.every_secs = None;
        }
        if let Some(t) = &req.timezone {
            goal.schedule.timezone = Some(t.trim().to_string()).filter(|t| !t.is_empty());
        }
        check_schedule(&goal.schedule)?;
        let now = crate::now_ms();
        if req.clear_script {
            goal.schedule.check = None;
        }
        let new_script = if req.restore_package {
            let pkg = goal
                .schedule
                .check
                .as_ref()
                .and_then(|c| c.package.clone())
                .ok_or_else(|| OpenDanError::InvalidArgument(format!("goal {goal_id} has no package check script to restore")))?;
            let path = self.layout.root.join(&pkg);
            let text = std::fs::read_to_string(&path).map_err(|e| OpenDanError::io(&path, e))?;
            Some((text, format!("package:{pkg}"), Some(pkg)))
        } else if let Some(text) = &req.script {
            let source = match &req.package {
                Some(p) => format!("package:{p}"),
                None => ck.via(),
            };
            let package = req
                .package
                .clone()
                .or_else(|| goal.schedule.check.as_ref().and_then(|c| c.package.clone()));
            Some((text.clone(), source, package))
        } else {
            None
        };
        if let Some((text, source, package)) = new_script {
            let unchanged = goal
                .schedule
                .check
                .as_ref()
                .is_some_and(|c| c.sha256 == script_digest(&text) && c.source == source);
            if !unchanged {
                goal.schedule.check = Some(self.store_script(goal_id, &text, source, package, now)?);
            }
        }
        if goal.schedule == before {
            return Ok(goal);
        }
        goal.updated_at_ms = now;
        let mut change = Self::change("check", caller, &ck, req.reason.clone());
        change.epoch = Some(goal.session.epoch);
        change.detail = json!({
            "every_secs": goal.schedule.every_secs,
            "cron": goal.schedule.cron,
            "timezone": goal.schedule.timezone,
            "script": goal.schedule.check,
        });
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn check_script(&self, goal_id: &str, version: Option<u64>) -> Result<Option<String>> {
        check_goal_id(goal_id)?;
        let version = match version {
            Some(v) => v,
            None => match self.read(goal_id)?.and_then(|g| g.schedule.check.map(|c| c.version)) {
                Some(v) => v,
                None => return Ok(None),
            },
        };
        let path = self.dir(goal_id).join("checks").join(format!("{version}.ts"));
        match std::fs::read_to_string(&path) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OpenDanError::io(&path, e)),
        }
    }

    async fn state(&self, goal_id: &str) -> Result<Option<GoalState>> {
        check_goal_id(goal_id)?;
        fsutil::read_json_opt(&self.state_file(goal_id))
    }

    async fn epoch_state(&self, goal_id: &str, epoch: u64) -> Result<Option<GoalState>> {
        check_goal_id(goal_id)?;
        let archived: Option<GoalState> = fsutil::read_json_opt(
            &self
                .dir(goal_id)
                .join("epochs")
                .join(epoch.to_string())
                .join("state.json"),
        )?;
        if archived.is_some() {
            return Ok(archived);
        }
        Ok(self.state(goal_id).await?.filter(|s| s.epoch == epoch))
    }

    async fn activations(&self, goal_id: &str, q: &ActivationQuery) -> Result<Vec<GoalActivation>> {
        check_goal_id(goal_id)?;
        let mut all: Vec<GoalActivation> =
            last_lines(&self.dir(goal_id).join("activations.jsonl"), 0)?;
        if let Some(e) = q.epoch {
            all.retain(|a| a.epoch == e);
        }
        if q.limit > 0 && all.len() > q.limit {
            all.drain(..all.len() - q.limit);
        }
        Ok(all)
    }

    async fn schedule(&self, goal_id: &str) -> Result<Option<GoalSchedule>> {
        check_goal_id(goal_id)?;
        fsutil::read_json_opt(&self.dir(goal_id).join("schedule.json"))
    }

    async fn write_state(&self, lease: &Lease, state: &GoalState) -> Result<()> {
        lease.check()?;
        check_goal_id(&state.goal_id)?;
        let _lock = self.lock(&state.goal_id)?;
        if let Some(cur) = fsutil::read_json_opt::<GoalState>(&self.state_file(&state.goal_id))? {
            if cur.epoch > state.epoch {
                return Err(OpenDanError::GoalConflict(format!(
                    "goal {} is in epoch {}; the state of epoch {} is not published",
                    state.goal_id, cur.epoch, state.epoch
                )));
            }
            if cur.epoch == state.epoch && cur.rev >= state.rev {
                return Ok(());
            }
        }
        lease.fenced(|| fsutil::atomic_replace_json(&self.state_file(&state.goal_id), state))
    }

    async fn append_activations(&self, lease: &Lease, records: &[GoalActivation]) -> Result<()> {
        lease.check()?;
        let Some(first) = records.first() else {
            return Ok(());
        };
        check_goal_id(&first.goal_id)?;
        let _lock = self.lock(&first.goal_id)?;
        let path = self.dir(&first.goal_id).join("activations.jsonl");
        let end = fsutil::file_len(&path).unwrap_or(0);
        let last: Option<GoalActivation> = if end > 0 {
            fsutil::tail_json(&path, end)?
        } else {
            None
        };
        let last_key = last.map(|a| (a.epoch, a.seq));
        let new: Vec<&GoalActivation> = records
            .iter()
            .filter(|r| last_key.is_none_or(|k| (r.epoch, r.seq) > k))
            .collect();
        if new.is_empty() {
            return Ok(());
        }
        let lines = fsutil::to_json_lines(&new)?;
        lease.fenced(|| fsutil::append_batch(&path, &lines).map(|_| ()))
    }

    async fn prepare_epoch(&self, goal_id: &str, epoch: u64) -> Result<GoalState> {
        let _lock = self.lock(goal_id)?;
        let goal = self.must(goal_id)?;
        let e = goal
            .epoch(epoch)
            .ok_or_else(|| OpenDanError::NotFound(format!("epoch {epoch} of goal {goal_id}")))?
            .clone();
        let now = crate::now_ms();
        let path = self.state_file(goal_id);
        let cur: Option<GoalState> = fsutil::read_json_opt(&path)?;
        let next = match cur {
            Some(s) if s.epoch == epoch => return Ok(s),
            Some(s) if s.epoch > epoch => {
                return Err(OpenDanError::GoalConflict(format!(
                    "goal {goal_id} is already in epoch {}",
                    s.epoch
                )))
            }
            Some(prev) => {
                let archive = self
                    .dir(goal_id)
                    .join("epochs")
                    .join(prev.epoch.to_string())
                    .join("state.json");
                fsutil::publish_noreplace_json(&archive, &prev)?;
                crate::fault::point("goal:after_epoch_archive");
                prev.for_epoch(&goal, epoch, e.keep, now)
            }
            None => GoalState::initial(&goal, epoch, now),
        };
        fsutil::atomic_replace_json(&path, &next)?;
        Ok(next)
    }

    async fn end_epoch(
        &self,
        goal_id: &str,
        epoch: u64,
        end: EpochEnd,
        reason: Option<String>,
    ) -> Result<GoalDefinition> {
        let _lock = self.lock(goal_id)?;
        let mut goal = self.must(goal_id)?;
        let now = crate::now_ms();
        let Some(e) = goal.epochs.iter_mut().find(|e| e.epoch == epoch) else {
            return Err(OpenDanError::NotFound(format!("epoch {epoch} of goal {goal_id}")));
        };
        if e.ended_at_ms.is_some() {
            return Ok(goal);
        }
        e.ended_at_ms = Some(now);
        if e.end.is_none() {
            e.end = Some(end);
        }
        if e.end_reason.is_none() {
            e.end_reason = reason.clone();
        }
        let end = e.end;
        goal.updated_at_ms = now;
        let mut change = GoalChange {
            seq: 0,
            at_ms: now,
            kind: "epoch_ended".into(),
            who: "opendan".into(),
            via: "opendan".into(),
            reason,
            revision: None,
            epoch: Some(epoch),
            detail: json!({ "end": end }),
        };
        change.epoch = Some(epoch);
        self.append_change(&mut goal, change)?;
        self.write_goal(&goal)?;
        Ok(goal)
    }

    async fn write_schedule(&self, schedule: &GoalSchedule) -> Result<()> {
        check_goal_id(&schedule.goal_id)?;
        let dir = self.dir(&schedule.goal_id);
        if !dir.join("goal.json").is_file() {
            return Err(OpenDanError::NotFound(format!("goal {}", schedule.goal_id)));
        }
        fsutil::atomic_replace_json(&dir.join("schedule.json"), schedule)
    }
}
