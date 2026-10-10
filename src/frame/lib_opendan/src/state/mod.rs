//! Agent State (§6): cross-session state on the AgentRoot, accessed only
//! through [`AgentStateClient`]. [`FsAgentStateClient`] reads and writes the
//! AgentRoot files directly and coordinates with file locks (a DFS mount
//! stays transparent to it); [`KrpcAgentStateClient`] reaches the Agent
//! State service of the OpenDAN process for callers without the AgentRoot.

mod activity;
mod artifacts;
mod behaviors;
mod cognition;
mod connect;
mod fs_client;
pub mod krpc;
mod locks;
pub mod perception;
mod registry;

use std::path::Path;

use async_trait::async_trait;

use crate::error::Result;
use crate::lock::{Acquire, Lease};
use crate::protocol::*;

pub use activity::{
    declared_refs, refs_overlap, render_active_sessions, ActiveSession, Relation,
    STALE_HEARTBEAT_MS,
};
pub use artifacts::{nearest_valid_base, DecideResult};
pub use behaviors::{
    freeze_behavior, freeze_config, parse_behavior, BehaviorCatalog, FsBehaviorCatalog,
    MemBehaviorCatalog, ROLE_SUPPLEMENT_FILE,
};
pub use cognition::{ConsolidationBatch, Hint, NotebookNote, RecallQuery};
pub use connect::{
    connect, register_in_process, unregister_in_process, ConnectOptions, ForwardingStateClient,
    StateLocator, WithBehaviors, ENV_AGENT_ROOT, ENV_AGENT_STATE_URL,
};
pub use fs_client::{AgentLayout, FsAgentStateClient};
pub use krpc::{KrpcAgentStateClient, KrpcTransport, StateTransport};
pub use perception::{run_digest, Backlog, BacklogItem, PERCEPTION_LOCK_TIMEOUT};

/// Session registry (session mgr, §6.2). The only entry point to discover
/// every session of an agent, wherever its directory lives.
#[async_trait]
pub trait SessionRegistry: Send + Sync {
    /// Called once by the creator; idempotent for the same identity.
    async fn register(&self, entry: RegistryEntry, who: &str) -> Result<RegistryEntry>;
    /// Driver only, after every commit; ignored unless `status.rev` is newer.
    async fn report_state(&self, lease: &Lease, sid: &str, status: SessionStatus) -> Result<bool>;
    async fn lookup(&self, sid: &str) -> Result<Option<RegistryEntry>>;
    async fn query(&self, q: &RegistryQuery) -> Result<Vec<RegistryEntry>>;
    /// Sub sessions of `parents` (`origin.parent_session`), by session id.
    async fn children_of(&self, parents: &[String]) -> Result<Vec<RegistryEntry>> {
        let mut out: Vec<RegistryEntry> = self
            .query(&RegistryQuery::default())
            .await?
            .into_iter()
            .filter(|e| {
                e.origin
                    .as_ref()
                    .and_then(|o| o.parent_session.as_ref())
                    .is_some_and(|p| parents.contains(p))
            })
            .collect();
        out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(out)
    }
    /// Driver only: the session directory moved.
    async fn update_location(&self, lease: &Lease, sid: &str, location: &Path) -> Result<()>;
    /// Mark entries whose location vanished as unreachable (never deletes).
    async fn verify(&self) -> Result<Vec<String>>;
    /// Post an input to a session's kmsg queue (+ wake event).
    /// Append a record to the session's input bus. Refused with
    /// `input_full` while the bus holds [`MAX_PENDING_INPUTS`] records that
    /// are not consumed yet (nothing is overwritten; retry later), with
    /// `queue_missing` while the queue is lost and not yet recreated by the
    /// driver (retry later), and with `session_readonly` for a session of an
    /// older protocol version.
    async fn post_input(&self, sid: &str, input: &PostedInput) -> Result<u64>;
}

/// Active session view (§6.7): other sessions that run and what they touch.
#[async_trait]
pub trait ActivityView: Send + Sync {
    async fn active(&self, me: Option<&RegistryEntry>, limit: usize) -> Result<Vec<ActiveSession>>;
}

/// Perception stream (§6.3): cross-session, append-only, no LLM on write.
#[async_trait]
pub trait Perception: Send + Sync {
    /// Driver only; retries are idempotent (records with `seq ≤` the last
    /// stored one are skipped).
    async fn append(&self, lease: &Lease, sid: &str, records: Vec<PerceptionRecord>) -> Result<u64>;
    async fn last_seq(&self, sid: &str) -> Result<u64>;
    async fn cursor(&self) -> Result<PerceptionCursor>;
    /// Derived from file sizes only.
    async fn backlog(&self, cursor: &PerceptionCursor) -> Result<Backlog>;
    async fn read(&self, item: &BacklogItem) -> Result<Vec<PerceptionRecord>>;
    /// Only the `self_improve` lease holder advances the cursor.
    async fn commit_cursor(&self, lease: &Lease, cursor: &PerceptionCursor) -> Result<()>;
}

/// Cognition facade (§6.4) over `agent_tool` memory / notebook.
#[async_trait]
pub trait Cognition: Send + Sync {
    /// Read-only; any session may call it.
    async fn recall_hints(&self, q: &RecallQuery) -> Result<Vec<Hint>>;
    /// Explicit declaration from an ordinary session ("remember this").
    async fn notebook_append(&self, note: &NotebookNote, who: &str) -> Result<()>;
    /// `self_improve` lease holder only: commit a consolidation batch, then
    /// advance the perception cursor (at-least-once consolidation).
    async fn commit_consolidation(
        &self,
        lease: &Lease,
        batch: &ConsolidationBatch,
        upto: &PerceptionCursor,
    ) -> Result<()>;
}

/// Artifact list (§6.5): registration and pointers only.
#[async_trait]
pub trait Artifacts: Send + Sync {
    async fn head(&self, aid: &str) -> Result<Option<ArtifactHead>>;
    async fn version(&self, aid: &str, ver: &str) -> Result<Option<ArtifactVersion>>;
    async fn versions(&self, aid: &str) -> Result<Vec<ArtifactVersion>>;
    async fn list(&self) -> Result<Vec<ArtifactHead>>;
    /// Write the session's own version file (`produced`); the session lease
    /// holder is the single writer. Creates `artifact.json` if missing.
    async fn register_version(
        &self,
        session_lease: &Lease,
        aid: &str,
        workspace: Option<WorkspaceRef>,
        version: ArtifactVersion,
    ) -> Result<()>;
    /// accept / discard under the `artifact:<aid>` lock (re-reads head and
    /// version inside the lock).
    async fn decide(
        &self,
        artifact_lease: &Lease,
        aid: &str,
        ver: &str,
        decision: &str,
    ) -> Result<DecideResult>;
}

/// Agent-level locks (session locks live on the session directory).
pub trait LockManager: Send + Sync {
    fn acquire(&self, resource: &str, holder: HolderInfo) -> Result<Acquire>;
    fn is_held(&self, resource: &str) -> Result<bool>;
}

/// The SDK boundary to Agent State (not a protocol; the protocol is the file
/// layout and the locks behind it).
pub trait AgentStateClient: Send + Sync {
    fn agent_did(&self) -> &str;
    /// Used in kevent / kmsg names (letters, digits, `_ - .`).
    fn agent_id(&self) -> &str;
    /// AgentRoot path when the client is file based.
    fn agent_root(&self) -> Option<&Path>;
    fn sessions(&self) -> &dyn SessionRegistry;
    fn activity(&self) -> &dyn ActivityView;
    fn perception(&self) -> &dyn Perception;
    fn cognition(&self) -> &dyn Cognition;
    fn artifacts(&self) -> &dyn Artifacts;
    fn locks(&self) -> &dyn LockManager;
    /// The agent's behaviors and identity text (frozen into a session when
    /// it is constructed).
    fn behaviors(&self) -> &dyn BehaviorCatalog;
}
