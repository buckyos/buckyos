//! Agent State over kRPC: the wire form of [`AgentStateClient`] for callers
//! that do not see the AgentRoot (tools in a container, WebUI, other apps).
//!
//! Both ends live here and are transport-free: [`serve_call`] dispatches one
//! call onto any `AgentStateClient` (the service wraps it with its
//! authentication), [`KrpcAgentStateClient`] turns the trait into calls over
//! a [`StateTransport`]. The truth stays the AgentRoot files of the serving
//! side; nothing is stored here.
//!
//! Only non-driver operations travel: reads and signed writes (`register`,
//! `post_input`, `notebook_append`, `decide`). What a driver writes under
//! its lease (`report_state`, `perception.append`, cursors, artifact
//! versions) and the Agent-level locks depend on flock and stay with the
//! processes that see the AgentRoot.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::lock::{Acquire, Lease};
use crate::protocol::*;

use super::*;

/// One call to the serving side.
#[async_trait]
pub trait StateTransport: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value>;
}

/// The error of a served call as it travels (`OpenDanError::to_json`).
pub fn error_to_wire(e: &OpenDanError) -> String {
    let mut v = e.to_json();
    match e {
        OpenDanError::InputFull { session_id, pending } => {
            v["session_id"] = json!(session_id);
            v["pending"] = json!(pending);
        }
        OpenDanError::QueueMissing { session_id } => v["session_id"] = json!(session_id),
        _ => {}
    }
    v.to_string()
}

/// The error a caller sees for a wire error (kinds a caller acts on keep
/// their variant).
pub fn error_from_wire(text: &str) -> OpenDanError {
    let Some(v) = text
        .find('{')
        .and_then(|i| serde_json::from_str::<Value>(&text[i..]).ok())
    else {
        return OpenDanError::Channel(text.to_string());
    };
    let message = v["message"].as_str().unwrap_or(text).to_string();
    let detail = |prefix: &str| {
        message
            .strip_prefix(prefix)
            .unwrap_or(&message)
            .to_string()
    };
    match v["kind"].as_str().unwrap_or_default() {
        "invalid_argument" => OpenDanError::InvalidArgument(detail("invalid argument: ")),
        "not_found" => OpenDanError::NotFound(detail("not found: ")),
        "input_full" => OpenDanError::InputFull {
            session_id: v["session_id"].as_str().unwrap_or_default().to_string(),
            pending: v["pending"].as_u64().unwrap_or(MAX_PENDING_INPUTS as u64) as usize,
        },
        "queue_missing" => OpenDanError::QueueMissing {
            session_id: v["session_id"].as_str().unwrap_or_default().to_string(),
        },
        "session_finished" => OpenDanError::SessionFinished(
            detail("session ").trim_end_matches(" is finished").to_string(),
        ),
        "session_id_conflict" => OpenDanError::SessionIdConflict(detail("session id conflict: ")),
        "artifact" => OpenDanError::Artifact(detail("artifact error: ")),
        "channel" => OpenDanError::Channel(detail("input channel error: ")),
        _ => OpenDanError::Other(message),
    }
}

fn arg<T: DeserializeOwned>(params: &Value, name: &str) -> Result<T> {
    serde_json::from_value(params.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| OpenDanError::InvalidArgument(format!("param `{name}`: {e}")))
}

fn out<T: Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| OpenDanError::Other(format!("encode result: {e}")))
}

/// Whether `method` changes state (a service authorizes it as a write).
pub fn is_write(method: &str) -> bool {
    matches!(
        method,
        "sessions.register"
            | "sessions.post_input"
            | "sessions.verify"
            | "cognition.notebook_append"
            | "artifacts.decide"
    )
}

/// Dispatch one Agent State call of `who` onto `agent`. `None`: not an
/// Agent State method.
pub async fn serve_call(
    agent: &dyn AgentStateClient,
    who: &str,
    method: &str,
    params: &Value,
) -> Option<Result<Value>> {
    Some(match method {
        "agent.info" => out(json!({
            "agent_did": agent.agent_did(),
            "agent_id": agent.agent_id(),
        })),
        "sessions.register" => match arg::<RegistryEntry>(params, "entry") {
            Ok(entry) if entry.created_by.principal != who => Err(OpenDanError::InvalidArgument(
                format!("entry.created_by is {}, the caller is {who}", entry.created_by.principal),
            )),
            Ok(entry) => agent.sessions().register(entry, who).await.and_then(out),
            Err(e) => Err(e),
        },
        "sessions.lookup" => match arg::<String>(params, "sid") {
            Ok(sid) => agent.sessions().lookup(&sid).await.and_then(out),
            Err(e) => Err(e),
        },
        "sessions.query" => match arg::<Option<RegistryQuery>>(params, "query") {
            Ok(q) => agent.sessions().query(&q.unwrap_or_default()).await.and_then(out),
            Err(e) => Err(e),
        },
        "sessions.children_of" => match arg::<Vec<String>>(params, "parents") {
            Ok(p) => agent.sessions().children_of(&p).await.and_then(out),
            Err(e) => Err(e),
        },
        "sessions.verify" => agent.sessions().verify().await.and_then(out),
        "sessions.post_input" => {
            match (arg::<String>(params, "sid"), arg::<PostedInput>(params, "input")) {
                (Ok(sid), Ok(mut input)) => {
                    // The record is signed by the caller the service saw.
                    input.from = who.to_string();
                    match input.validate() {
                        Ok(()) => agent.sessions().post_input(&sid, &input).await.and_then(out),
                        Err(e) => Err(e),
                    }
                }
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        "activity.active" => {
            let limit = arg::<Option<usize>>(params, "limit").ok().flatten().unwrap_or(20);
            match arg::<Option<String>>(params, "me") {
                Ok(me) => {
                    let me = match me {
                        Some(sid) => match agent.sessions().lookup(&sid).await {
                            Ok(e) => e,
                            Err(e) => return Some(Err(e)),
                        },
                        None => None,
                    };
                    agent.activity().active(me.as_ref(), limit).await.and_then(out)
                }
                Err(e) => Err(e),
            }
        }
        "perception.last_seq" => match arg::<String>(params, "sid") {
            Ok(sid) => agent.perception().last_seq(&sid).await.and_then(out),
            Err(e) => Err(e),
        },
        "perception.cursor" => agent.perception().cursor().await.and_then(out),
        "perception.backlog" => match arg::<PerceptionCursor>(params, "cursor") {
            Ok(c) => agent.perception().backlog(&c).await.and_then(out),
            Err(e) => Err(e),
        },
        "perception.read" => match arg::<BacklogItem>(params, "item") {
            Ok(i) => agent.perception().read(&i).await.and_then(out),
            Err(e) => Err(e),
        },
        "cognition.recall_hints" => match arg::<RecallQuery>(params, "query") {
            Ok(q) => agent.cognition().recall_hints(&q).await.and_then(out),
            Err(e) => Err(e),
        },
        "cognition.notebook_append" => match arg::<NotebookNote>(params, "note") {
            Ok(n) => agent.cognition().notebook_append(&n, who).await.and_then(out),
            Err(e) => Err(e),
        },
        "artifacts.head" => match arg::<String>(params, "aid") {
            Ok(aid) => agent.artifacts().head(&aid).await.and_then(out),
            Err(e) => Err(e),
        },
        "artifacts.version" => match (arg::<String>(params, "aid"), arg::<String>(params, "ver")) {
            (Ok(aid), Ok(ver)) => agent.artifacts().version(&aid, &ver).await.and_then(out),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "artifacts.versions" => match arg::<String>(params, "aid") {
            Ok(aid) => agent.artifacts().versions(&aid).await.and_then(out),
            Err(e) => Err(e),
        },
        "artifacts.list" => agent.artifacts().list().await.and_then(out),
        "artifacts.decide" => {
            match (
                arg::<String>(params, "aid"),
                arg::<String>(params, "ver"),
                arg::<String>(params, "decision"),
            ) {
                (Ok(aid), Ok(ver), Ok(decision)) => {
                    decide_as(agent, who, &aid, &ver, &decision).await.and_then(out)
                }
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
            }
        }
        "behaviors.identity" => agent.behaviors().identity().await.and_then(out),
        "behaviors.get" => match arg::<String>(params, "name") {
            Ok(n) => agent.behaviors().get(&n).await.and_then(out),
            Err(e) => Err(e),
        },
        "behaviors.list" => agent.behaviors().list().await.and_then(out),
        "behaviors.revision" => agent.behaviors().revision().await.and_then(out),
        _ => return None,
    })
}

/// accept / discard on behalf of `who`: the serving side takes the
/// `artifact:<aid>` lock for the call.
async fn decide_as(
    agent: &dyn AgentStateClient,
    who: &str,
    aid: &str,
    ver: &str,
    decision: &str,
) -> Result<DecideResult> {
    let resource = format!("artifact:{aid}");
    let holder = HolderInfo {
        runner_id: crate::ids::new_runner_id(),
        principal: who.to_string(),
        host: Some(crate::runtime::native_host_id()),
        pid: std::process::id(),
        runtime_id: None,
    };
    match agent.locks().acquire(&resource, holder)? {
        Acquire::Acquired(lease) => agent.artifacts().decide(&lease, aid, ver, decision).await,
        Acquire::Busy(holder) => Err(OpenDanError::Busy {
            resource,
            holder: holder.and_then(|h| serde_json::to_value(h).ok()),
        }),
    }
}

pub const ENV_AGENT_STATE_TOKEN: &str = "OPENDAN_AGENT_STATE_TOKEN";

/// kRPC transport to the service at `endpoint`. The caller's identity is
/// its BuckyOS session token: a fixed one, or the token of the process's
/// BuckyOS runtime read for every call (it is renewed while the process
/// lives).
pub struct KrpcTransport {
    endpoint: String,
    token: Option<String>,
}

impl KrpcTransport {
    pub fn new(endpoint: &str, token: Option<String>) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            token,
        }
    }

    async fn token(&self) -> Option<String> {
        if self.token.is_some() {
            return self.token.clone();
        }
        if let Ok(rt) = buckyos_api::get_buckyos_api_runtime() {
            return Some(rt.get_session_token().await).filter(|t| !t.is_empty());
        }
        std::env::var(ENV_AGENT_STATE_TOKEN).ok().filter(|t| !t.is_empty())
    }
}

#[async_trait]
impl StateTransport for KrpcTransport {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let client = ::kRPC::kRPC::new(&self.endpoint, self.token().await);
        client.call(method, params).await.map_err(|e| match e {
            ::kRPC::RPCErrors::ReasonError(text) => error_from_wire(&text),
            other => OpenDanError::Channel(format!("{}: {other}", self.endpoint)),
        })
    }
}

/// [`AgentStateClient`] of a process that does not see the AgentRoot.
pub struct KrpcAgentStateClient {
    transport: Arc<dyn StateTransport>,
    agent_did: String,
    agent_id: String,
}

fn driver_only(what: &str) -> OpenDanError {
    OpenDanError::Other(format!(
        "{what} is a driver's write under its lease: it needs the AgentRoot (files + flock) and is not served over kRPC"
    ))
}

impl KrpcAgentStateClient {
    pub fn new(agent_did: &str, transport: Arc<dyn StateTransport>) -> Self {
        Self {
            transport,
            agent_did: agent_did.to_string(),
            agent_id: crate::ids::agent_id_from_did(agent_did),
        }
    }

    /// Connect and check that the service serves `agent_did`.
    pub async fn connect(agent_did: &str, transport: Arc<dyn StateTransport>) -> Result<Self> {
        let info = transport.call("agent.info", json!({})).await?;
        let served = info["agent_did"].as_str().unwrap_or_default();
        if served != agent_did {
            return Err(OpenDanError::NotFound(format!(
                "agent state of {agent_did}: the service serves {served}"
            )));
        }
        let mut c = Self::new(agent_did, transport);
        if let Some(id) = info["agent_id"].as_str() {
            c.agent_id = id.to_string();
        }
        Ok(c)
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let v = self.transport.call(method, params).await?;
        serde_json::from_value(v)
            .map_err(|e| OpenDanError::Other(format!("{method}: unexpected result: {e}")))
    }

    /// accept / discard: the service takes the artifact lock for the call.
    pub async fn decide(&self, aid: &str, ver: &str, decision: &str) -> Result<DecideResult> {
        self.call("artifacts.decide", json!({ "aid": aid, "ver": ver, "decision": decision }))
            .await
    }
}

#[async_trait]
impl SessionRegistry for KrpcAgentStateClient {
    async fn register(&self, entry: RegistryEntry, _who: &str) -> Result<RegistryEntry> {
        self.call("sessions.register", json!({ "entry": entry })).await
    }
    async fn report_state(&self, _lease: &Lease, _sid: &str, _status: SessionStatus) -> Result<bool> {
        Err(driver_only("report_state"))
    }
    async fn lookup(&self, sid: &str) -> Result<Option<RegistryEntry>> {
        self.call("sessions.lookup", json!({ "sid": sid })).await
    }
    async fn query(&self, q: &RegistryQuery) -> Result<Vec<RegistryEntry>> {
        self.call("sessions.query", json!({ "query": q })).await
    }
    async fn children_of(&self, parents: &[String]) -> Result<Vec<RegistryEntry>> {
        self.call("sessions.children_of", json!({ "parents": parents })).await
    }
    async fn update_location(&self, _lease: &Lease, _sid: &str, _location: &Path) -> Result<()> {
        Err(driver_only("update_location"))
    }
    async fn verify(&self) -> Result<Vec<String>> {
        self.call("sessions.verify", json!({})).await
    }
    async fn post_input(&self, sid: &str, input: &PostedInput) -> Result<u64> {
        self.call("sessions.post_input", json!({ "sid": sid, "input": input })).await
    }
}

#[async_trait]
impl ActivityView for KrpcAgentStateClient {
    async fn active(&self, me: Option<&RegistryEntry>, limit: usize) -> Result<Vec<ActiveSession>> {
        self.call(
            "activity.active",
            json!({ "me": me.map(|e| e.session_id.clone()), "limit": limit }),
        )
        .await
    }
}

#[async_trait]
impl Perception for KrpcAgentStateClient {
    async fn append(&self, _lease: &Lease, _sid: &str, _records: Vec<PerceptionRecord>) -> Result<u64> {
        Err(driver_only("perception.append"))
    }
    async fn last_seq(&self, sid: &str) -> Result<u64> {
        self.call("perception.last_seq", json!({ "sid": sid })).await
    }
    async fn cursor(&self) -> Result<PerceptionCursor> {
        self.call("perception.cursor", json!({})).await
    }
    async fn backlog(&self, cursor: &PerceptionCursor) -> Result<Backlog> {
        self.call("perception.backlog", json!({ "cursor": cursor })).await
    }
    async fn read(&self, item: &BacklogItem) -> Result<Vec<PerceptionRecord>> {
        self.call("perception.read", json!({ "item": item })).await
    }
    async fn commit_cursor(&self, _lease: &Lease, _cursor: &PerceptionCursor) -> Result<()> {
        Err(driver_only("perception.commit_cursor"))
    }
}

#[async_trait]
impl Cognition for KrpcAgentStateClient {
    async fn recall_hints(&self, q: &RecallQuery) -> Result<Vec<Hint>> {
        self.call("cognition.recall_hints", json!({ "query": q })).await
    }
    async fn notebook_append(&self, note: &NotebookNote, _who: &str) -> Result<()> {
        self.call("cognition.notebook_append", json!({ "note": note })).await
    }
    async fn commit_consolidation(
        &self,
        _lease: &Lease,
        _batch: &ConsolidationBatch,
        _upto: &PerceptionCursor,
    ) -> Result<()> {
        Err(driver_only("commit_consolidation"))
    }
}

#[async_trait]
impl Artifacts for KrpcAgentStateClient {
    async fn head(&self, aid: &str) -> Result<Option<ArtifactHead>> {
        self.call("artifacts.head", json!({ "aid": aid })).await
    }
    async fn version(&self, aid: &str, ver: &str) -> Result<Option<ArtifactVersion>> {
        self.call("artifacts.version", json!({ "aid": aid, "ver": ver })).await
    }
    async fn versions(&self, aid: &str) -> Result<Vec<ArtifactVersion>> {
        self.call("artifacts.versions", json!({ "aid": aid })).await
    }
    async fn list(&self) -> Result<Vec<ArtifactHead>> {
        self.call("artifacts.list", json!({})).await
    }
    async fn register_version(
        &self,
        _session_lease: &Lease,
        _aid: &str,
        _workspace: Option<WorkspaceRef>,
        _version: ArtifactVersion,
    ) -> Result<()> {
        Err(driver_only("artifacts.register_version"))
    }
    async fn decide(
        &self,
        _artifact_lease: &Lease,
        aid: &str,
        ver: &str,
        decision: &str,
    ) -> Result<DecideResult> {
        KrpcAgentStateClient::decide(self, aid, ver, decision).await
    }
}

impl LockManager for KrpcAgentStateClient {
    fn acquire(&self, _resource: &str, _holder: HolderInfo) -> Result<Acquire> {
        Err(driver_only("an Agent-level lock"))
    }
    fn is_held(&self, _resource: &str) -> Result<bool> {
        Err(driver_only("an Agent-level lock"))
    }
}

#[async_trait]
impl BehaviorCatalog for KrpcAgentStateClient {
    async fn identity(&self) -> Result<IdentityText> {
        self.call("behaviors.identity", json!({})).await
    }
    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        self.call("behaviors.get", json!({ "name": name })).await
    }
    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        self.call("behaviors.list", json!({})).await
    }
    async fn revision(&self) -> Result<String> {
        self.call("behaviors.revision", json!({})).await
    }
}

impl AgentStateClient for KrpcAgentStateClient {
    fn agent_did(&self) -> &str {
        &self.agent_did
    }
    fn agent_id(&self) -> &str {
        &self.agent_id
    }
    fn agent_root(&self) -> Option<&Path> {
        None
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
    fn locks(&self) -> &dyn LockManager {
        self
    }
    fn behaviors(&self) -> &dyn BehaviorCatalog {
        self
    }
}
