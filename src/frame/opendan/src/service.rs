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

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_http_server::{serve_http_by_rpc_handler, HttpServer, ServerError, ServerResult, StreamInfo};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use kRPC::{RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult};
use libopendan::host::Supervisor;
use libopendan::protocol::*;
use libopendan::state::krpc::{error_to_wire, is_write, serve_call};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionDir};
use serde::Serialize;
use serde_json::{json, Value};

use crate::home::Home;
use crate::ui::UiModule;

pub const SERVICE_PATH: &str = "/kapi/opendan";

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
                let user = token
                    .sub
                    .clone()
                    .ok_or_else(|| RPCErrors::InvalidToken("missing session subject".to_string()))?;
                if &user != owner && user != "root" {
                    return Err(RPCErrors::NoPermission(format!(
                        "{user} is not the owner of this agent"
                    )));
                }
                Ok(match token.appid.as_deref().filter(|a| !a.is_empty()) {
                    Some(app) => format!("app:{app}@{user}"),
                    None => format!("did:bns:{user}"),
                })
            }
        }
    }

    fn profile(&self) -> Value {
        let (owner_did, desktop_url) = match &self.access {
            Access::Zone { owner, zone_host, .. } => (
                Some(format!("did:bns:{owner}")),
                zone_host.as_ref().map(|h| format!("https://{h}")),
            ),
            Access::Open { .. } => (None, None),
        };
        self.home.profile(self.agent.as_ref(), owner_did, desktop_url)
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
            return result;
        }
        match method {
            "session.read" => self.read_session(params).await,
            "session.stop" => {
                let sid: String = param(params, "sid")?;
                let reason = param::<Option<String>>(params, "reason")?;
                self.post_control(who, &sid, ControlCommand::Stop { reason }).await
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
            "agent.profile" => Ok(self.profile()),
            "agent.profile_set" => {
                self.home.set_profile(params)?;
                Ok(self.profile())
            }
            "usage.models" => self.home.usage_models(self.agent.as_ref()).await,
            "ui.bindings" => self.home.ui_bindings(self.agent.as_ref()).await,
            "loader.status" => Ok(json!({
                "agent_did": self.info.agent_did,
                "agent_id": self.info.agent_id,
                "who": self.info.who,
                "agent_root": self.info.agent_root,
                "started_at_ms": self.info.started_at_ms,
                "now_ms": libopendan::now_ms(),
                "modules": *self.info.modules.lock().expect("modules"),
                "hosted": self.supervisor.status(),
                "ui": self.ui.as_ref().map(|u| u.status()),
                "errors": *self.info.errors.lock().expect("errors"),
            })),
            other => Err(OpenDanError::InvalidArgument(format!("unknown method `{other}`"))),
        }
    }
}

/// Methods outside the Agent State facade that change something.
fn is_operation(method: &str) -> bool {
    matches!(
        method,
        "session.stop" | "session.decide" | "session.post" | "agent.profile_set"
    )
}

#[async_trait]
impl RPCHandler for StateService {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse, RPCErrors> {
        let who = self.caller(&req, ip_from).await?;
        if is_write(&req.method) || is_operation(&req.method) {
            log::info!("service: {} by {who}", req.method);
        }
        match self.call(&who, &req.method, &req.params).await {
            Ok(v) => Ok(RPCResponse::new(RPCResult::Success(v), req.seq)),
            Err(e) => Err(RPCErrors::ReasonError(error_to_wire(&e))),
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
