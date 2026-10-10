//! Locating an agent's state (xAgent §7): nothing is configured, a caller
//! only gives the agent DID and its own identity.
//!
//! Resolution order: in-process registry → AgentRoot on this machine → kRPC
//! (the Agent State service of the OpenDAN process hosting the agent, see
//! [`super::krpc`]). [`ForwardingStateClient`] owns no state and passes
//! every call on: it proves the runner depends on the trait only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use buckyos_api::msg_queue::MsgQueueClient;

use crate::channel::Waker;
use crate::error::{OpenDanError, Result};
use crate::lock::{Acquire, Lease};
use crate::protocol::*;

use super::*;

pub const ENV_AGENT_ROOT: &str = "OPENDAN_AGENT_ROOT";
pub const ENV_AGENT_STATE_URL: &str = "OPENDAN_AGENT_STATE_URL";

/// Where an agent's state lives.
#[derive(Clone)]
pub enum StateLocator {
    /// The host process holds the components.
    InProcess(Arc<dyn AgentStateClient>),
    /// This machine sees the AgentRoot: files + flock.
    AgentRoot(PathBuf),
    /// Another process / container serves it.
    Krpc { endpoint: String },
}

/// What a file based client needs besides the path.
#[derive(Clone, Default)]
pub struct ConnectOptions {
    pub hint: Option<StateLocator>,
    pub queue: Option<Arc<MsgQueueClient>>,
    pub waker: Option<Arc<dyn Waker>>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<dyn AgentStateClient>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<dyn AgentStateClient>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Make `client` the state of its agent for this process (embedding hosts).
pub fn register_in_process(client: Arc<dyn AgentStateClient>) {
    registry()
        .lock()
        .expect("state registry")
        .insert(client.agent_did().to_string(), client);
}

pub fn unregister_in_process(agent_did: &str) {
    registry().lock().expect("state registry").remove(agent_did);
}

/// `~/.opendan/agents.toml`: `"<agent did>" = "<agent root>"`.
fn mapped_root(agent_did: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = Path::new(&home).join(".opendan").join("agents.toml");
    let table: toml::Table = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    table.get(agent_did)?.as_str().map(PathBuf::from)
}

/// Connect to the state of `agent_did` as `who`.
pub async fn connect(
    agent_did: &str,
    who: &str,
    options: ConnectOptions,
) -> Result<Arc<dyn AgentStateClient>> {
    let _ = who;
    let open_root = |root: &Path| -> Result<Arc<dyn AgentStateClient>> {
        Ok(Arc::new(FsAgentStateClient::open(
            root,
            agent_did,
            options.queue.clone(),
            options.waker.clone(),
        )?))
    };
    match &options.hint {
        Some(StateLocator::InProcess(c)) => return Ok(c.clone()),
        Some(StateLocator::AgentRoot(root)) => return open_root(root),
        Some(StateLocator::Krpc { endpoint }) => return krpc(agent_did, endpoint).await,
        None => {}
    }
    if let Some(c) = registry().lock().expect("state registry").get(agent_did) {
        return Ok(c.clone());
    }
    if let Some(root) = std::env::var_os(ENV_AGENT_ROOT).filter(|r| !r.is_empty()) {
        return open_root(Path::new(&root));
    }
    if let Some(root) = mapped_root(agent_did) {
        return open_root(&root);
    }
    if let Ok(endpoint) = std::env::var(ENV_AGENT_STATE_URL) {
        return krpc(agent_did, &endpoint).await;
    }
    Err(OpenDanError::NotFound(format!(
        "agent state of {agent_did}: no in-process client, ${ENV_AGENT_ROOT}, ~/.opendan/agents.toml entry or ${ENV_AGENT_STATE_URL}"
    )))
}

async fn krpc(agent_did: &str, endpoint: &str) -> Result<Arc<dyn AgentStateClient>> {
    let transport = Arc::new(super::krpc::KrpcTransport::new(endpoint, None));
    Ok(Arc::new(
        super::krpc::KrpcAgentStateClient::connect(agent_did, transport).await?,
    ))
}

/// Every facade call is forwarded to `inner`. It exposes no AgentRoot path,
/// like a remote client.
pub struct ForwardingStateClient {
    inner: Arc<dyn AgentStateClient>,
    /// Whether callers may see the AgentRoot path (a remote client cannot).
    expose_root: bool,
}

impl ForwardingStateClient {
    pub fn new(inner: Arc<dyn AgentStateClient>) -> Self {
        Self {
            inner,
            expose_root: false,
        }
    }

    /// Same machine: tools of the session still find the AgentRoot.
    pub fn with_root(mut self) -> Self {
        self.expose_root = true;
        self
    }
}

#[async_trait]
impl SessionRegistry for ForwardingStateClient {
    async fn register(&self, entry: RegistryEntry, who: &str) -> Result<RegistryEntry> {
        self.inner.sessions().register(entry, who).await
    }
    async fn report_state(&self, lease: &Lease, sid: &str, status: SessionStatus) -> Result<bool> {
        self.inner.sessions().report_state(lease, sid, status).await
    }
    async fn lookup(&self, sid: &str) -> Result<Option<RegistryEntry>> {
        self.inner.sessions().lookup(sid).await
    }
    async fn query(&self, q: &RegistryQuery) -> Result<Vec<RegistryEntry>> {
        self.inner.sessions().query(q).await
    }
    async fn children_of(&self, parents: &[String]) -> Result<Vec<RegistryEntry>> {
        self.inner.sessions().children_of(parents).await
    }
    async fn update_location(&self, lease: &Lease, sid: &str, location: &Path) -> Result<()> {
        self.inner.sessions().update_location(lease, sid, location).await
    }
    async fn verify(&self) -> Result<Vec<String>> {
        self.inner.sessions().verify().await
    }
    async fn post_input(&self, sid: &str, input: &PostedInput) -> Result<u64> {
        self.inner.sessions().post_input(sid, input).await
    }
}

#[async_trait]
impl ActivityView for ForwardingStateClient {
    async fn active(&self, me: Option<&RegistryEntry>, limit: usize) -> Result<Vec<ActiveSession>> {
        self.inner.activity().active(me, limit).await
    }
}

#[async_trait]
impl Perception for ForwardingStateClient {
    async fn append(&self, lease: &Lease, sid: &str, records: Vec<PerceptionRecord>) -> Result<u64> {
        self.inner.perception().append(lease, sid, records).await
    }
    async fn last_seq(&self, sid: &str) -> Result<u64> {
        self.inner.perception().last_seq(sid).await
    }
    async fn cursor(&self) -> Result<PerceptionCursor> {
        self.inner.perception().cursor().await
    }
    async fn backlog(&self, cursor: &PerceptionCursor) -> Result<Backlog> {
        self.inner.perception().backlog(cursor).await
    }
    async fn read(&self, item: &BacklogItem) -> Result<Vec<PerceptionRecord>> {
        self.inner.perception().read(item).await
    }
    async fn commit_cursor(&self, lease: &Lease, cursor: &PerceptionCursor) -> Result<()> {
        self.inner.perception().commit_cursor(lease, cursor).await
    }
}

#[async_trait]
impl Cognition for ForwardingStateClient {
    async fn recall_hints(&self, q: &RecallQuery) -> Result<Vec<Hint>> {
        self.inner.cognition().recall_hints(q).await
    }
    async fn notebook_append(&self, note: &NotebookNote, who: &str) -> Result<()> {
        self.inner.cognition().notebook_append(note, who).await
    }
    async fn commit_consolidation(
        &self,
        lease: &Lease,
        batch: &ConsolidationBatch,
        upto: &PerceptionCursor,
    ) -> Result<()> {
        self.inner
            .cognition()
            .commit_consolidation(lease, batch, upto)
            .await
    }
}

#[async_trait]
impl Artifacts for ForwardingStateClient {
    async fn head(&self, aid: &str) -> Result<Option<ArtifactHead>> {
        self.inner.artifacts().head(aid).await
    }
    async fn version(&self, aid: &str, ver: &str) -> Result<Option<ArtifactVersion>> {
        self.inner.artifacts().version(aid, ver).await
    }
    async fn versions(&self, aid: &str) -> Result<Vec<ArtifactVersion>> {
        self.inner.artifacts().versions(aid).await
    }
    async fn list(&self) -> Result<Vec<ArtifactHead>> {
        self.inner.artifacts().list().await
    }
    async fn register_version(
        &self,
        session_lease: &Lease,
        aid: &str,
        workspace: Option<WorkspaceRef>,
        version: ArtifactVersion,
    ) -> Result<()> {
        self.inner
            .artifacts()
            .register_version(session_lease, aid, workspace, version)
            .await
    }
    async fn decide(
        &self,
        artifact_lease: &Lease,
        aid: &str,
        ver: &str,
        decision: &str,
    ) -> Result<DecideResult> {
        self.inner
            .artifacts()
            .decide(artifact_lease, aid, ver, decision)
            .await
    }
}

impl LockManager for ForwardingStateClient {
    fn acquire(&self, resource: &str, holder: HolderInfo) -> Result<Acquire> {
        self.inner.locks().acquire(resource, holder)
    }
    fn is_held(&self, resource: &str) -> Result<bool> {
        self.inner.locks().is_held(resource)
    }
}

#[async_trait]
impl BehaviorCatalog for ForwardingStateClient {
    async fn identity(&self) -> Result<IdentityText> {
        self.inner.behaviors().identity().await
    }
    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        self.inner.behaviors().get(name).await
    }
    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        self.inner.behaviors().list().await
    }
    async fn revision(&self) -> Result<String> {
        self.inner.behaviors().revision().await
    }
}

impl AgentStateClient for ForwardingStateClient {
    fn agent_did(&self) -> &str {
        self.inner.agent_did()
    }
    fn agent_id(&self) -> &str {
        self.inner.agent_id()
    }
    fn agent_root(&self) -> Option<&Path> {
        if self.expose_root {
            self.inner.agent_root()
        } else {
            None
        }
    }
    fn sessions(&self) -> &dyn SessionRegistry {
        self
    }
    fn activity(&self) -> &dyn ActivityView {
        self
    }
    fn perception(&self) -> &dyn Perception {
        self
    }
    fn cognition(&self) -> &dyn Cognition {
        self
    }
    fn artifacts(&self) -> &dyn Artifacts {
        self
    }
    fn workspaces(&self) -> &dyn WorkspaceManager {
        self.inner.workspaces()
    }
    fn locks(&self) -> &dyn LockManager {
        self
    }
    fn behaviors(&self) -> &dyn BehaviorCatalog {
        self
    }
}

/// An agent state whose behavior catalog is supplied by the host process
/// (the other facades stay with `inner`).
pub struct WithBehaviors {
    inner: Arc<dyn AgentStateClient>,
    behaviors: Arc<dyn BehaviorCatalog>,
}

impl WithBehaviors {
    pub fn new(inner: Arc<dyn AgentStateClient>, behaviors: Arc<dyn BehaviorCatalog>) -> Self {
        Self { inner, behaviors }
    }
}

impl AgentStateClient for WithBehaviors {
    fn agent_did(&self) -> &str {
        self.inner.agent_did()
    }
    fn agent_id(&self) -> &str {
        self.inner.agent_id()
    }
    fn agent_root(&self) -> Option<&Path> {
        self.inner.agent_root()
    }
    fn sessions(&self) -> &dyn SessionRegistry {
        self.inner.sessions()
    }
    fn activity(&self) -> &dyn ActivityView {
        self.inner.activity()
    }
    fn perception(&self) -> &dyn Perception {
        self.inner.perception()
    }
    fn cognition(&self) -> &dyn Cognition {
        self.inner.cognition()
    }
    fn artifacts(&self) -> &dyn Artifacts {
        self.inner.artifacts()
    }
    fn workspaces(&self) -> &dyn WorkspaceManager {
        self.inner.workspaces()
    }
    fn locks(&self) -> &dyn LockManager {
        self.inner.locks()
    }
    fn behaviors(&self) -> &dyn BehaviorCatalog {
        self.behaviors.as_ref()
    }
}
