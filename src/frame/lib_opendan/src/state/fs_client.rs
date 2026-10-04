//! `FsAgentStateClient`: Agent State directly on AgentRoot files (§6.1).
//!
//! ```text
//! <agent_root>/
//!   state/sessions/<sid>.json            registry
//!   state/perception/<sid>.jsonl         perception
//!   state/perception/.cursor.json        consolidation cursor
//!   state/artifacts/<aid>/artifact.json  artifact head
//!   state/artifacts/<aid>/versions/v-<sid>.json
//!   memory/ notebook/ attention_signals/ cognition (agent_tool)
//!   .locks/self_improve.lease  .locks/artifact/<aid>.lease
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use buckyos_api::msg_queue::MsgQueueClient;

use crate::channel::Waker;
use crate::error::{OpenDanError, Result};

use super::activity::FsActivity;
use super::artifacts::FsArtifacts;
use super::behaviors::FsBehaviorCatalog;
use super::cognition::FsCognition;
use super::locks::FsLocks;
use super::perception::FsPerception;
use super::registry::FsRegistry;
use super::*;

/// Paths of the AgentRoot layout.
#[derive(Debug, Clone)]
pub struct AgentLayout {
    pub root: PathBuf,
}

impl AgentLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("state").join("sessions")
    }

    pub fn registry_entry(&self, sid: &str) -> PathBuf {
        self.sessions_dir().join(format!("{sid}.json"))
    }

    pub fn unreachable_mark(&self, sid: &str) -> PathBuf {
        self.sessions_dir().join(".unreachable").join(sid)
    }

    pub fn perception_dir(&self) -> PathBuf {
        self.root.join("state").join("perception")
    }

    pub fn perception_file(&self, sid: &str) -> PathBuf {
        self.perception_dir().join(format!("{sid}.jsonl"))
    }

    pub fn perception_cursor(&self) -> PathBuf {
        self.perception_dir().join(".cursor.json")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.root.join("state").join("artifacts")
    }

    pub fn artifact_head(&self, aid: &str) -> PathBuf {
        self.artifacts_dir().join(aid).join("artifact.json")
    }

    pub fn artifact_version(&self, aid: &str, ver: &str) -> PathBuf {
        self.artifacts_dir()
            .join(aid)
            .join("versions")
            .join(format!("{ver}.json"))
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.root.join(".locks")
    }

    /// Lock file of an agent-level resource.
    pub fn lock_path(&self, resource: &str) -> Result<PathBuf> {
        if resource == "self_improve" {
            return Ok(self.locks_dir().join("self_improve.lease"));
        }
        if let Some(aid) = resource.strip_prefix("artifact:") {
            crate::ids::validate_session_id(aid)
                .map_err(|_| OpenDanError::InvalidArgument(format!("bad artifact id `{aid}`")))?;
            return Ok(self.locks_dir().join("artifact").join(format!("{aid}.lease")));
        }
        Err(OpenDanError::InvalidArgument(format!(
            "unknown agent lock resource `{resource}`"
        )))
    }

    pub fn default_sessions_parent(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    pub fn notebook_dir(&self) -> PathBuf {
        self.root.join("notebook")
    }

    pub fn workspace_dir(&self, wid: &str) -> PathBuf {
        self.root.join("workspace").join(wid)
    }
}

/// File-based `AgentStateClient` (V3). Requires the runner to see the
/// AgentRoot path (A8).
pub struct FsAgentStateClient {
    layout: AgentLayout,
    did: String,
    id: String,
    registry: FsRegistry,
    activity: FsActivity,
    perception: FsPerception,
    cognition: FsCognition,
    artifacts: FsArtifacts,
    locks: FsLocks,
    behaviors: FsBehaviorCatalog,
}

impl FsAgentStateClient {
    /// Open the Agent State of `agent_did` at `agent_root`. `queue` is used
    /// by `post_input`; `waker` publishes wake events after posting.
    pub fn open(
        agent_root: impl AsRef<Path>,
        agent_did: &str,
        queue: Option<Arc<MsgQueueClient>>,
        waker: Option<Arc<dyn Waker>>,
    ) -> Result<Self> {
        let root = agent_root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root).map_err(|e| OpenDanError::io(&root, e))?;
        let layout = AgentLayout::new(root);
        for d in [
            layout.sessions_dir(),
            layout.perception_dir(),
            layout.artifacts_dir(),
            layout.locks_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| OpenDanError::io(&d, e))?;
        }
        let id = crate::ids::agent_id_from_did(agent_did);
        Ok(Self {
            registry: FsRegistry::new(layout.clone(), queue, waker),
            activity: FsActivity::new(layout.clone()),
            perception: FsPerception::new(layout.clone()),
            cognition: FsCognition::new(layout.clone()),
            artifacts: FsArtifacts::new(layout.clone()),
            locks: FsLocks::new(layout.clone()),
            behaviors: FsBehaviorCatalog::new(layout.root.clone()),
            layout,
            did: agent_did.to_string(),
            id,
        })
    }

    /// Override the agent id used in queue / event names.
    pub fn with_agent_id(mut self, id: &str) -> Self {
        self.id = crate::ids::sanitize_segment(id);
        self
    }

    pub fn layout(&self) -> &AgentLayout {
        &self.layout
    }
}

impl AgentStateClient for FsAgentStateClient {
    fn agent_did(&self) -> &str {
        &self.did
    }

    fn agent_id(&self) -> &str {
        &self.id
    }

    fn agent_root(&self) -> Option<&Path> {
        Some(&self.layout.root)
    }

    fn sessions(&self) -> &dyn SessionRegistry {
        &self.registry
    }

    fn activity(&self) -> &dyn ActivityView {
        &self.activity
    }

    fn perception(&self) -> &dyn Perception {
        &self.perception
    }

    fn cognition(&self) -> &dyn Cognition {
        &self.cognition
    }

    fn artifacts(&self) -> &dyn Artifacts {
        &self.artifacts
    }

    fn locks(&self) -> &dyn LockManager {
        &self.locks
    }

    fn behaviors(&self) -> &dyn BehaviorCatalog {
        &self.behaviors
    }
}
