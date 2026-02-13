use ::kRPC::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanAgentInfo {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanWorkspaceInfo {
    pub workspace_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_db_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worklog_db_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanWorklogItem {
    pub log_id: String,
    pub log_type: String,
    pub status: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanTodoItem {
    pub todo_id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_in_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_in_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanSubAgentInfo {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanAgentListResult {
    pub items: Vec<OpenDanAgentInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanWorkspaceWorklogsResult {
    pub items: Vec<OpenDanWorklogItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanWorkspaceTodosResult {
    pub items: Vec<OpenDanTodoItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanWorkspaceSubAgentsResult {
    pub items: Vec<OpenDanSubAgentInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanListAgentsReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_sub_agents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl OpenDanListAgentsReq {
    pub fn new(
        status: Option<String>,
        include_sub_agents: Option<bool>,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Self {
        Self {
            status,
            include_sub_agents,
            limit,
            cursor,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse OpenDanListAgentsReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDanGetAgentReq {
    pub agent_id: String,
}

impl OpenDanGetAgentReq {
    pub fn new(agent_id: String) -> Self {
        Self { agent_id }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse OpenDanGetAgentReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDanGetWorkspaceReq {
    pub agent_id: String,
}

impl OpenDanGetWorkspaceReq {
    pub fn new(agent_id: String) -> Self {
        Self { agent_id }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse OpenDanGetWorkspaceReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanListWorkspaceWorklogsReq {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl OpenDanListWorkspaceWorklogsReq {
    pub fn new(
        agent_id: String,
        log_type: Option<String>,
        status: Option<String>,
        step_id: Option<String>,
        keyword: Option<String>,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Self {
        Self {
            agent_id,
            log_type,
            status,
            step_id,
            keyword,
            limit,
            cursor,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse OpenDanListWorkspaceWorklogsReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanListWorkspaceTodosReq {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_closed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl OpenDanListWorkspaceTodosReq {
    pub fn new(
        agent_id: String,
        status: Option<String>,
        include_closed: Option<bool>,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Self {
        Self {
            agent_id,
            status,
            include_closed,
            limit,
            cursor,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse OpenDanListWorkspaceTodosReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenDanListWorkspaceSubAgentsReq {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl OpenDanListWorkspaceSubAgentsReq {
    pub fn new(
        agent_id: String,
        include_disabled: Option<bool>,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Self {
        Self {
            agent_id,
            include_disabled,
            limit,
            cursor,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse OpenDanListWorkspaceSubAgentsReq: {}",
                error
            ))
        })
    }
}

pub enum OpenDanClient {
    InProcess(Box<dyn OpenDanHandler>),
    KRPC(Box<kRPC>),
}

impl OpenDanClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn OpenDanHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn list_agents(
        &self,
        status: Option<&str>,
        include_sub_agents: Option<bool>,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<OpenDanAgentListResult> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let req = OpenDanListAgentsReq::new(
                    status.map(|value| value.to_string()),
                    include_sub_agents,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                handler.handle_list_agents(req, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanListAgentsReq::new(
                    status.map(|value| value.to_string()),
                    include_sub_agents,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanListAgentsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_agents", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_agents response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn get_agent(&self, agent_id: &str) -> Result<OpenDanAgentInfo> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_agent(agent_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanGetAgentReq::new(agent_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanGetAgentReq: {}",
                        error
                    ))
                })?;
                let result = client.call("get_agent", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse get_agent response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn get_workspace(&self, agent_id: &str) -> Result<OpenDanWorkspaceInfo> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_workspace(agent_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanGetWorkspaceReq::new(agent_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanGetWorkspaceReq: {}",
                        error
                    ))
                })?;
                let result = client.call("get_workspace", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse get_workspace response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_workspace_worklogs(
        &self,
        agent_id: &str,
        log_type: Option<&str>,
        status: Option<&str>,
        step_id: Option<&str>,
        keyword: Option<&str>,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<OpenDanWorkspaceWorklogsResult> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let req = OpenDanListWorkspaceWorklogsReq::new(
                    agent_id.to_string(),
                    log_type.map(|value| value.to_string()),
                    status.map(|value| value.to_string()),
                    step_id.map(|value| value.to_string()),
                    keyword.map(|value| value.to_string()),
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                handler.handle_list_workspace_worklogs(req, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanListWorkspaceWorklogsReq::new(
                    agent_id.to_string(),
                    log_type.map(|value| value.to_string()),
                    status.map(|value| value.to_string()),
                    step_id.map(|value| value.to_string()),
                    keyword.map(|value| value.to_string()),
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanListWorkspaceWorklogsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_workspace_worklogs", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_workspace_worklogs response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_workspace_todos(
        &self,
        agent_id: &str,
        status: Option<&str>,
        include_closed: Option<bool>,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<OpenDanWorkspaceTodosResult> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let req = OpenDanListWorkspaceTodosReq::new(
                    agent_id.to_string(),
                    status.map(|value| value.to_string()),
                    include_closed,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                handler.handle_list_workspace_todos(req, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanListWorkspaceTodosReq::new(
                    agent_id.to_string(),
                    status.map(|value| value.to_string()),
                    include_closed,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanListWorkspaceTodosReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_workspace_todos", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_workspace_todos response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_workspace_sub_agents(
        &self,
        agent_id: &str,
        include_disabled: Option<bool>,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<OpenDanWorkspaceSubAgentsResult> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let req = OpenDanListWorkspaceSubAgentsReq::new(
                    agent_id.to_string(),
                    include_disabled,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                handler.handle_list_workspace_sub_agents(req, ctx).await
            }
            Self::KRPC(client) => {
                let req = OpenDanListWorkspaceSubAgentsReq::new(
                    agent_id.to_string(),
                    include_disabled,
                    limit,
                    cursor.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize OpenDanListWorkspaceSubAgentsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_workspace_sub_agents", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_workspace_sub_agents response: {}",
                        error
                    ))
                })
            }
        }
    }
}

#[async_trait]
pub trait OpenDanHandler: Send + Sync {
    async fn handle_list_agents(
        &self,
        request: OpenDanListAgentsReq,
        ctx: RPCContext,
    ) -> Result<OpenDanAgentListResult>;

    async fn handle_get_agent(&self, agent_id: &str, ctx: RPCContext) -> Result<OpenDanAgentInfo>;

    async fn handle_get_workspace(
        &self,
        agent_id: &str,
        ctx: RPCContext,
    ) -> Result<OpenDanWorkspaceInfo>;

    async fn handle_list_workspace_worklogs(
        &self,
        request: OpenDanListWorkspaceWorklogsReq,
        ctx: RPCContext,
    ) -> Result<OpenDanWorkspaceWorklogsResult>;

    async fn handle_list_workspace_todos(
        &self,
        request: OpenDanListWorkspaceTodosReq,
        ctx: RPCContext,
    ) -> Result<OpenDanWorkspaceTodosResult>;

    async fn handle_list_workspace_sub_agents(
        &self,
        request: OpenDanListWorkspaceSubAgentsReq,
        ctx: RPCContext,
    ) -> Result<OpenDanWorkspaceSubAgentsResult>;
}

pub struct OpenDanServerHandler<T: OpenDanHandler>(pub T);

impl<T: OpenDanHandler> OpenDanServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: OpenDanHandler> RPCHandler for OpenDanServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "list_agents" => {
                let request = OpenDanListAgentsReq::from_json(req.params)?;
                let result = self.0.handle_list_agents(request, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_agent" => {
                let request = OpenDanGetAgentReq::from_json(req.params)?;
                let result = self.0.handle_get_agent(&request.agent_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_workspace" => {
                let request = OpenDanGetWorkspaceReq::from_json(req.params)?;
                let result = self.0.handle_get_workspace(&request.agent_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_workspace_worklogs" => {
                let request = OpenDanListWorkspaceWorklogsReq::from_json(req.params)?;
                let result = self.0.handle_list_workspace_worklogs(request, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_workspace_todos" => {
                let request = OpenDanListWorkspaceTodosReq::from_json(req.params)?;
                let result = self.0.handle_list_workspace_todos(request, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_workspace_sub_agents" => {
                let request = OpenDanListWorkspaceSubAgentsReq::from_json(req.params)?;
                let result = self.0.handle_list_workspace_sub_agents(request, ctx).await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}
