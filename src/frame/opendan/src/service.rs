//! The Agent State service: a layer of authentication and forwarding around
//! the process's [`AgentStateClient`] (the truth stays the AgentRoot files),
//! plus what the WebUI needs to observe a session and the Loader itself.
//!
//! Besides the Agent State facade (`libopendan::state::krpc`) it serves:
//!
//! - `session.read {sid, report?, worklog?}`: registry entry, state, worklog
//!   tail, report, configuration summary, runs, hosting state. Read-only;
//!   there is no write interface to a session directory.
//! - `session.stop {sid, reason?}`, `session.decide {sid, decision, note?}`,
//!   `session.post {sid, text}`: the three operations of the WebUI; each is
//!   a record posted to the session's input bus, nothing else.
//! - `loader.status`: hosting state, modules, recent errors.
//! - `agent.profile`, `agent.profile_set`, `usage.models`, `ui.bindings`:
//!   the home page ([`crate::home`]).
//! - `goals.*` of the facade, plus `goals.activate` / `goals.post` (a run
//!   opportunity / a message for a Goal); a Goal changed here is reconciled
//!   by the host right away.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{
    load_agent_permission_mode, validate_verify_hub_token_claims, AgentPermissionMode,
    AppInstanceId, AuthTarget, TokenPrincipalKind, TokenUse,
};
use buckyos_http_server::{serve_http_by_rpc_handler, HttpServer, ServerError, ServerResult, StreamInfo};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use kRPC::{RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult};
use libopendan::host::Supervisor;
use libopendan::protocol::*;
use libopendan::state::krpc::{error_to_wire, is_write, serve_call};
use libopendan::session_context::{is_session_method, is_session_token, SessionService};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionDir};
use serde::Serialize;
use serde_json::{json, Value};

use crate::home::Home;
use crate::ui::UiModule;

pub const SERVICE_PATH: &str = "/kapi/opendan";
pub const WEB_API_PATH: &str = "/api/opendan";

/// Who may call the service.
#[derive(Debug, Clone)]
pub enum Access {
    /// Development host outside a zone: every caller is `who`.
    Open { who: String },
    /// Callers present a BuckyOS session token issued by verify-hub; the
    /// agent's owner (through any app) and the zone's root are admitted.
    /// `trust_loopback` (local debugging only): a caller on the loopback
    /// interface without a token is the owner. A gateway on this host
    /// forwards from loopback too.
    Zone {
        owner: String,
        app_id: String,
        trust_loopback: bool,
        /// Host name of the zone: where the desktop (MessageHub) is served.
        zone_host: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleStatus {
    pub name: String,
    pub enabled: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoaderError {
    pub at_ms: u64,
    pub source: String,
    pub message: String,
}

/// What the Loader shows about itself (memory only).
pub struct LoaderInfo {
    pub agent_did: String,
    pub agent_id: String,
    pub who: String,
    pub agent_root: String,
    pub started_at_ms: u64,
    pub modules: Mutex<Vec<ModuleStatus>>,
    pub errors: Mutex<Vec<LoaderError>>,
}

impl LoaderInfo {
    pub fn error(&self, source: &str, message: impl Into<String>) {
        let mut errors = self.errors.lock().expect("errors");
        errors.push(LoaderError {
            at_ms: libopendan::now_ms(),
            source: source.to_string(),
            message: message.into(),
        });
        let extra = errors.len().saturating_sub(50);
        errors.drain(..extra);
    }
}

pub struct StateService {
    pub agent: Arc<dyn AgentStateClient>,
    pub supervisor: Arc<Supervisor>,
    pub info: Arc<LoaderInfo>,
    pub ui: Option<Arc<UiModule>>,
    pub access: Access,
    pub home: Home,
    /// Session processes on other runtimes: their session tokens and
    /// methods (`libopendan::session_context`).
    pub sessions: Option<Arc<SessionService>>,
}

fn param<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T, OpenDanError> {
    serde_json::from_value(params.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| OpenDanError::InvalidArgument(format!("param `{name}`: {e}")))
}

impl StateService {
    async fn caller(&self, req: &RPCRequest, ip_from: IpAddr) -> Result<String, RPCErrors> {
        match &self.access {
            Access::Open { who } => Ok(who.clone()),
            Access::Zone {
                owner,
                app_id,
                trust_loopback,
                ..
            } => {
                if *trust_loopback && req.token.is_none() && ip_from.is_loopback() {
                    return Ok(format!("did:bns:{owner}"));
                }
                let token = req.token.as_deref().ok_or_else(|| {
                    RPCErrors::NoPermission("a session token is required".to_string())
                })?;
                let runtime = buckyos_api::get_buckyos_api_runtime()?;
                let token = runtime.verify_trusted_session_token(token).await?;
                let claims = validate_verify_hub_token_claims(&token, TokenUse::Session)?;
                let user = token
                    .sub
                    .clone()
                    .ok_or_else(|| RPCErrors::InvalidToken("missing session subject".to_string()))?;
                if &user != owner && user != "root" {
                    return Err(RPCErrors::NoPermission(format!(
                        "{user} is not the owner of this agent"
                    )));
                }
                if claims.principal_kind == TokenPrincipalKind::App {
                    let AuthTarget::App { app_instance_id } = &claims.target else {
                        return Err(RPCErrors::InvalidToken(
                            "an app principal requires an app target".to_string(),
                        ));
                    };
                    let client = runtime.get_system_config_client().await?;
                    let mode = load_agent_permission_mode(client.as_ref(), app_instance_id).await?;
                    authorize_agent_method(mode, app_instance_id, app_id, owner, &req.method)?;
                }
                Ok(match token.appid.as_deref().filter(|a| !a.is_empty()) {
                    Some(app) => format!("app:{app}@{user}"),
                    None => format!("did:bns:{user}"),
                })
            }
        }
    }

    async fn read_session(&self, params: &Value) -> Result<Value, OpenDanError> {
        let sid: String = param(params, "sid")?;
        let report = param::<Option<bool>>(params, "report")?.unwrap_or(true);
        let worklog = param::<Option<usize>>(params, "worklog")?.unwrap_or(40).min(500);
        let view = libopendan::read_session(self.agent.as_ref(), &sid, false, report, worklog).await?;
        let mut out = serde_json::to_value(&view)
            .map_err(|e| OpenDanError::Other(format!("encode session: {e}")))?;
        if let Ok(sd) = SessionDir::open(&view.entry.location) {
            if let Ok(cfg) = sd.config() {
                out["config"] = json!({
                    "session": cfg.session,
                    "runtime": cfg.runtime,
                    "workspace": cfg.workspace,
                    "workspace_binding": cfg.workspace_binding,
                    "subscriptions": cfg.subscriptions,
                    "channels": cfg.channels,
                    "behavior": cfg.prompt.behavior,
                    "frozen": cfg.prompt.frozen.as_ref().map(|f| json!({
                        "catalog_rev": f.catalog_rev,
                        "frozen_at_ms": f.frozen_at_ms,
                        "frozen_by": f.frozen_by,
                        "behaviors": f.behaviors.keys().collect::<Vec<_>>(),
                    })),
                });
            }
            out["binding"] = json!(sd.binding_opt().ok().flatten());
            out["statistics"] = json!(sd.statistics().ok());
            out["runs"] = json!(sd.runs().list().unwrap_or_default());
            out["lease_holder"] = json!(sd.holder());
        }
        out["children"] = json!(self
            .agent
            .sessions()
            .children_of(&[sid.clone()])
            .await?
            .into_iter()
            .map(|e| e.session_id)
            .collect::<Vec<_>>());
        out["hosted"] = json!(self
            .supervisor
            .status()
            .into_iter()
            .find(|h| h.session_id == sid));
        Ok(out)
    }

    async fn read_worklog(&self, params: &Value) -> Result<Value, OpenDanError> {
        let sid: String = param(params, "sid")?;
        let turn: u64 = param(params, "turn")?;
        if turn == 0 {
            return Err(OpenDanError::InvalidArgument("turn must be positive".into()));
        }
        let entry = self.agent.sessions().lookup(&sid).await?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
        let sd = SessionDir::open(&entry.location)?;
        let state = sd.state()?;
        let page = sd.worklog().page(
            state.worklog.committed_bytes,
            turn,
            param(params, "before")?,
            param(params, "after")?,
            param::<Option<usize>>(params, "limit")?.unwrap_or(50),
        )?;
        let children = self.agent.sessions().children_of(&[sid.clone()]).await?;
        let links: Vec<Value> = children.iter().filter_map(|child| {
            let origin = child.origin.as_ref()?;
            Some(json!({
                "created_by_call": origin.created_by_call.as_ref()?,
                "session_id": child.session_id,
                "name": child.objective,
            }))
        }).collect();
        Ok(json!({
            "agent_did": self.agent.agent_did(),
            "session_id": sid,
            "turn": turn,
            "entries": page.entries,
            "next_before": page.next_before,
            "next_after": page.next_after,
            "committed": state.worklog.committed_bytes,
            "complete": state.turn_seq >= turn && state.open_turn.as_ref().is_none_or(|open| open.index != turn),
            "task_id": state.turn_tasks.iter().find(|t| t.turn == turn).map(|t| &t.task_id),
            "children": links,
        }))
    }

    async fn post_control(
        &self,
        who: &str,
        sid: &str,
        command: ControlCommand,
    ) -> Result<Value, OpenDanError> {
        let key = format!("ctl:{}", libopendan::ids::new_runner_id());
        let index = self
            .agent
            .sessions()
            .post_input(sid, &PostedInput::control(who, key, command))
            .await?;
        self.host(sid, "control").await;
        Ok(json!({ "index": index }))
    }

    /// New input reached a session of this agent: make sure it is advanced
    /// (a session someone else drives is theirs to advance).
    async fn host(&self, sid: &str, reason: &str) {
        match self.supervisor.ensure_task(sid, reason).await {
            Ok(()) | Err(OpenDanError::NotDriver { .. }) => {}
            Err(e) => log::warn!("service: session {sid} not hosted: {e}"),
        }
    }

    async fn call(&self, who: &str, method: &str, params: &Value) -> Result<Value, OpenDanError> {
        if let Some(result) = serve_call(self.agent.as_ref(), who, method, params).await {
            if method == "sessions.post_input" && result.is_ok() {
                if let Ok(sid) = param::<String>(params, "sid") {
                    self.host(&sid, "input").await;
                }
            }
            if method.starts_with("goals.") && is_write(method) {
                // The host acts on the change now (its session, its epoch).
                let id = param::<String>(params, "goal_id")
                    .ok()
                    .or_else(|| result.as_ref().ok().and_then(|g| g["goal_id"].as_str().map(str::to_string)));
                if let Some(id) = id {
                    if let Err(e) = self.supervisor.goal_changed(&id).await {
                        log::warn!("service: goal {id} not reconciled yet: {e}");
                    }
                }
            }
            return result;
        }
        match method {
            "session.read" => self.read_session(params).await,
            "session.worklog" => self.read_worklog(params).await,
            "session.stop" => {
                let sid: String = param(params, "sid")?;
                let reason = param::<Option<String>>(params, "reason")?;
                // A Goal Session only stops its open Activation; the session
                // itself ends with its Goal.
                let scope = param::<Option<StopScope>>(params, "scope")?.unwrap_or_default();
                self.post_control(who, &sid, ControlCommand::Stop { reason, scope }).await
            }
            "session.decide" => {
                let sid: String = param(params, "sid")?;
                let decision: String = param(params, "decision")?;
                if !matches!(decision.as_str(), "accept" | "discard") {
                    return Err(OpenDanError::InvalidArgument(
                        "decision is accept | discard".to_string(),
                    ));
                }
                let note = param::<Option<String>>(params, "note")?;
                self.post_control(
                    who,
                    &sid,
                    ControlCommand::Decide {
                        decision,
                        by: who.to_string(),
                        note,
                    },
                )
                .await
            }
            "session.post" => {
                let sid: String = param(params, "sid")?;
                let text: String = param(params, "text")?;
                let input = PostedInput::text(who, self.agent.agent_did(), text)?;
                let index = self.agent.sessions().post_input(&sid, &input).await?;
                self.host(&sid, "input").await;
                Ok(json!({ "index": index, "key": input.key }))
            }
            "agent.profile" => self.home.profile(self.agent.as_ref()).await,
            "agent.profile_set" => {
                self.home.set_profile(params).await?;
                self.home.profile(self.agent.as_ref()).await
            }
            "usage.models" => self.home.usage_models(self.agent.as_ref()).await,
            "ui.bindings" => self.home.ui_bindings(self.agent.as_ref()).await,
            "home.workspaces" => self.home.workspaces(self.agent.as_ref()).await,
            "runtimes.settings" => self.home.runtime_settings().await,
            "runtimes.settings_set" => {
                let settings = self.home.set_runtime_settings(params).await?;
                // The projection follows the settings at once (the watcher
                // would catch up later).
                if let Some(root) = crate::home::agent_root_of(self.agent.as_ref()) {
                    crate::loader::project_runtimes(self.agent.as_ref(), &root, &settings, who).await;
                    tokio::spawn(crate::loader::check_runtimes(self.agent.clone()));
                }
                self.home.runtime_settings().await
            }
            "goals.activate" => {
                let id: String = param(params, "goal_id")?;
                let reason = param::<Option<String>>(params, "reason")?
                    .unwrap_or_else(|| format!("activated by {who}"));
                let request = param::<Option<String>>(params, "request_id")?
                    .unwrap_or_else(libopendan::ids::new_runner_id);
                let index = self.supervisor.goal_activate(&id, &request, &reason).await?;
                Ok(json!({ "index": index, "key": libopendan::goal::activate_key(&id, &request) }))
            }
            "goals.post" => {
                let id: String = param(params, "goal_id")?;
                let text: String = param(params, "text")?;
                let from = match param::<Option<String>>(params, "session")? {
                    Some(s) => format!("session:{s}"),
                    None => who.to_string(),
                };
                let (index, key) = self.supervisor.goal_feedback(&id, &text, &from).await?;
                Ok(json!({ "index": index, "key": key }))
            }
            "loader.status" => {
                let authentication = match buckyos_api::get_buckyos_api_runtime() {
                    Ok(runtime) => Some(runtime.get_authentication_status().await),
                    Err(_) => None,
                };
                Ok(json!({
                "agent_did": self.info.agent_did,
                "agent_id": self.info.agent_id,
                "who": self.info.who,
                "agent_root": self.info.agent_root,
                "started_at_ms": self.info.started_at_ms,
                "now_ms": libopendan::now_ms(),
                "authentication": authentication,
                "modules": *self.info.modules.lock().expect("modules"),
                "hosted": self.supervisor.status(),
                "ui": self.ui.as_ref().map(|u| u.status()),
                "goals": self.supervisor.goals().map(|g| g.status()),
                "errors": *self.info.errors.lock().expect("errors"),
                }))
            }
            other => Err(OpenDanError::InvalidArgument(format!("unknown method `{other}`"))),
        }
    }
}

fn authorize_agent_method(
    mode: Option<AgentPermissionMode>,
    caller: &AppInstanceId,
    target_app_id: &str,
    target_owner: &str,
    method: &str,
) -> Result<(), RPCErrors> {
    if mode != Some(AgentPermissionMode::ReadOnly) || is_read(method) {
        return Ok(());
    }
    if caller.app_id().as_str() == target_app_id
        && caller.owner_user_id() == target_owner
        && method != "agent.profile_set"
        && (is_write(method) || is_operation(method))
    {
        return Ok(());
    }
    Err(RPCErrors::NoPermission(format!(
        "read-only Agent cannot call {method} on this Agent"
    )))
}

fn is_read(method: &str) -> bool {
    matches!(
        method,
        "agent.info"
            | "sessions.lookup"
            | "sessions.query"
            | "sessions.children_of"
            | "activity.active"
            | "perception.last_seq"
            | "perception.cursor"
            | "perception.backlog"
            | "perception.read"
            | "cognition.recall_hints"
            | "artifacts.head"
            | "artifacts.version"
            | "artifacts.versions"
            | "artifacts.list"
            | "workspaces.lookup"
            | "workspaces.query"
            | "workspaces.list"
            | "workspaces.runtime_impact"
            | "runtimes.list"
            | "runtimes.lookup"
            | "behaviors.identity"
            | "behaviors.get"
            | "behaviors.list"
            | "behaviors.revision"
            | "behaviors.cmd_skill"
            | "skills.list"
            | "goals.list"
            | "goals.get"
            | "goals.revision"
            | "goals.changes"
            | "goals.state"
            | "goals.epoch_state"
            | "goals.activations"
            | "goals.schedule"
            | "notify.state"
            | "notify.list"
            | "session.read"
            | "session.worklog"
            | "agent.profile"
            | "usage.models"
            | "ui.bindings"
            | "home.workspaces"
            | "runtimes.settings"
            | "loader.status"
    )
}

/// Methods outside the Agent State facade that change something.
fn is_operation(method: &str) -> bool {
    matches!(
        method,
        "session.stop"
            | "session.decide"
            | "session.post"
            | "agent.profile_set"
            | "runtimes.settings_set"
            | "goals.activate"
            | "goals.post"
    )
}

impl StateService {
    /// A call with a session token: only that session's own methods.
    async fn session_call(&self, req: &RPCRequest, token: &str) -> Result<RPCResponse, RPCErrors> {
        // Refusals in the facade's wire form: a Session process tells them
        // apart from transport failures by kind.
        let forbidden = |m: String| RPCErrors::ReasonError(error_to_wire(&OpenDanError::Forbidden(m)));
        let sessions = self
            .sessions
            .as_ref()
            .ok_or_else(|| forbidden("this host issues no session tokens".to_string()))?;
        let claims = sessions
            .authenticate(self.agent.agent_id(), token)
            .map_err(|e| forbidden(e.to_string()))?;
        if !is_session_method(&req.method) {
            return Err(forbidden(format!("a session token cannot call {}", req.method)));
        }
        log::info!("service: {} by session {}", req.method, claims.session_id);
        match sessions.call(self.agent.as_ref(), &claims, &req.method, &req.params).await {
            Ok(v) => Ok(RPCResponse::new(RPCResult::Success(v), req.seq)),
            Err(e) => Err(RPCErrors::ReasonError(error_to_wire(&e))),
        }
    }
}

#[async_trait]
impl RPCHandler for StateService {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse, RPCErrors> {
        if let Some(token) = req.token.as_deref().filter(|t| is_session_token(t)) {
            return self.session_call(&req, token).await;
        }
        let who = self.caller(&req, ip_from).await?;
        if is_write(&req.method) || is_operation(&req.method) {
            log::info!("service: {} by {who}", req.method);
        }
        match self.call(&who, &req.method, &req.params).await {
            Ok(v) => Ok(RPCResponse::new(RPCResult::Success(v), req.seq)),
            Err(e) => {
                if let OpenDanError::QueueMissing { session_id } = &e {
                    // Its driver recreates the queue when it drives the
                    // session; the caller retries.
                    self.host(session_id, "input queue missing").await;
                }
                Err(RPCErrors::ReasonError(error_to_wire(&e)))
            }
        }
    }
}

#[async_trait]
impl HttpServer for StateService {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == http::Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        // What a page needs before it can ask the zone for a session token:
        // the app it is served by (`null` outside a zone: no login).
        if *req.method() == http::Method::GET {
            let app_id = match &self.access {
                Access::Zone { app_id, .. } => Some(app_id.as_str()),
                Access::Open { .. } => None,
            };
            let body = json!({ "app_id": app_id }).to_string();
            return http::Response::builder()
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::CACHE_CONTROL, "no-store")
                .body(Full::new(Bytes::from(body)).map_err(|e| match e {}).boxed())
                .map_err(|e| {
                    buckyos_http_server::server_err!(
                        buckyos_http_server::ServerErrorCode::InvalidData,
                        "{e}"
                    )
                });
        }
        Err(buckyos_http_server::server_err!(
            buckyos_http_server::ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "opendan-agent-state".to_string()
    }

    fn http_version(&self) -> http::Version {
        http::Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    const OWN_APP: &str = "reader.test.buckyos.io";
    const OTHER_APP: &str = "helper.test.buckyos.io";

    fn authorize(mode: Option<AgentPermissionMode>, method: &str, target: &str) -> bool {
        let caller = format!("{OWN_APP}@alice").parse::<AppInstanceId>().unwrap();
        authorize_agent_method(mode, &caller, target, "alice", method).is_ok()
    }

    #[test]
    fn read_only_agent_can_observe_own_and_other_agents() {
        for method in [
            "agent.info", "sessions.lookup", "sessions.query", "sessions.children_of",
            "activity.active", "perception.last_seq", "perception.cursor", "perception.backlog",
            "perception.read", "cognition.recall_hints", "artifacts.head", "artifacts.version",
            "artifacts.versions", "artifacts.list", "workspaces.lookup", "workspaces.query",
            "workspaces.list", "workspaces.runtime_impact", "runtimes.list", "runtimes.lookup",
            "behaviors.identity", "behaviors.get", "behaviors.list", "behaviors.revision",
            "behaviors.cmd_skill", "skills.list", "goals.list", "goals.get", "goals.revision",
            "goals.changes", "goals.state", "goals.epoch_state", "goals.activations",
            "goals.schedule", "notify.state", "notify.list", "session.read", "session.worklog",
            "agent.profile", "usage.models", "ui.bindings", "home.workspaces", "loader.status",
        ] {
            for target in [OWN_APP, OTHER_APP] {
                assert!(authorize(Some(AgentPermissionMode::ReadOnly), method, target), "{method}");
            }
        }
    }

    #[test]
    fn read_only_agent_writes_only_its_own_state() {
        for method in [
            "sessions.reserve_id", "sessions.register", "sessions.post_input", "sessions.verify",
            "cognition.notebook_append", "artifacts.decide", "workspaces.create",
            "workspaces.import", "workspaces.discover", "workspaces.check", "workspaces.update",
            "workspaces.archive", "workspaces.unregister", "workspaces.set_runtime_available",
            "runtimes.register", "runtimes.update", "runtimes.check", "runtimes.unregister",
            "goals.create", "goals.revise", "goals.set_status", "goals.reopen", "goals.restart",
            "notify.send", "notify.cancel", "notify.answer", "skills.install", "skills.remove",
            "session.stop", "session.decide", "session.post", "goals.activate", "goals.post",
        ] {
            assert!(authorize(Some(AgentPermissionMode::ReadOnly), method, OWN_APP), "{method}");
            assert!(!authorize(Some(AgentPermissionMode::ReadOnly), method, OTHER_APP), "{method}");
        }
        let caller = format!("{OWN_APP}@bob").parse::<AppInstanceId>().unwrap();
        assert!(authorize_agent_method(
            Some(AgentPermissionMode::ReadOnly), &caller, OWN_APP, "alice", "session.post"
        ).is_err());
    }

    #[test]
    fn read_only_agent_cannot_edit_profiles_or_call_unclassified_methods() {
        for method in [
            "agent.profile_set", "goals.write_state", "goals.append_activations",
            "goals.prepare_epoch", "goals.end_epoch", "goals.write_schedule",
            "agent.future_get", "sessions.delete", "unknown",
        ] {
            for target in [OWN_APP, OTHER_APP] {
                assert!(!authorize(Some(AgentPermissionMode::ReadOnly), method, target), "{method}");
            }
        }
    }

    #[test]
    fn read_write_agents_and_normal_apps_keep_existing_access() {
        for mode in [None, Some(AgentPermissionMode::ReadWrite)] {
            for target in [OWN_APP, OTHER_APP] {
                assert!(authorize(mode, "agent.profile_set", target));
                assert!(authorize(mode, "session.post", target));
            }
        }
    }
}
