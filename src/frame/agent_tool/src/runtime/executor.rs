use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{
    agent_daemon_byte_stream, AgentDaemonOnlineTarget, AgentDaemonStreamRoute, ExecutorChannel,
    ExecutorWriteMode,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::node::{read_locator_checked, NodeClient, NodeExecutorConnector, NodeIo, RemoteOp, TokenFuture};
use crate::llm_bash::RunBindingSlot;
use crate::{AgentTool, AgentToolError, ToolCtx, TypedTool, TypedToolHandle};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutorTarget {
    pub executor_id: String,
    pub generation: u64,
    pub profile_id: String,
    pub store_id: String,
}

pub struct ExecutorDirectory {
    endpoint: Option<String>,
    token: Arc<dyn Fn() -> TokenFuture + Send + Sync>,
}

impl ExecutorDirectory {
    pub fn new(endpoint: String, token: Arc<dyn Fn() -> TokenFuture + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            endpoint: Some(endpoint),
            token,
        })
    }

    pub fn from_buckyos_runtime() -> Arc<Self> {
        Arc::new(Self {
            endpoint: None,
            token: Arc::new(|| {
                Box::pin(async {
                    let runtime =
                        buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
                    Ok(runtime.get_session_token().await)
                })
            }),
        })
    }

    async fn call(&self, method: &str, params: Value, token: String) -> Result<Value, String> {
        let endpoint = match &self.endpoint {
            Some(endpoint) => endpoint.clone(),
            None => buckyos_api::get_buckyos_api_runtime()
                .map_err(|e| e.to_string())?
                .get_zone_service_url("control-panel", false)
                .await
                .map_err(|e| e.to_string())?,
        };
        let rpc = kRPC::kRPC::new_with_timeout_secs(&endpoint, Some(token), 15);
        rpc.call(method, params).await.map_err(|e| e.to_string())
    }

    pub async fn list(&self) -> Result<Vec<AgentDaemonOnlineTarget>, String> {
        let response = self
            .call("executor.online.list", json!({}), (self.token)().await?)
            .await?;
        serde_json::from_value(response.get("executors").cloned().unwrap_or(Value::Null))
            .map_err(|e| e.to_string())
    }

    pub async fn connector(
        self: &Arc<Self>,
        target: ExecutorTarget,
    ) -> Result<Arc<WssExecutorConnector>, String> {
        if target.executor_id.is_empty()
            || target.profile_id.is_empty()
            || target.store_id.is_empty()
            || target.generation == 0
        {
            return Err(
                "executor target requires executor_id, active generation, profile_id and store_id"
                    .into(),
            );
        }
        Ok(Arc::new(WssExecutorConnector {
            directory: self.clone(),
            target,
            token: (self.token)().await?,
        }))
    }
}

pub struct WssExecutorConnector {
    directory: Arc<ExecutorDirectory>,
    target: ExecutorTarget,
    token: String,
}

#[async_trait]
impl NodeExecutorConnector for WssExecutorConnector {
    async fn connect(&self, executor_id: &str) -> std::io::Result<Box<dyn NodeIo>> {
        let error =
            |message: String| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, message);
        if executor_id != self.target.executor_id {
            return Err(error("executor target mismatch".into()));
        }
        let value = self.directory.call("executor.stream.open", json!({
            "executor_id": self.target.executor_id, "generation": self.target.generation, "profile_id": self.target.profile_id,
        }), self.token.clone()).await.map_err(error)?;
        let route: AgentDaemonStreamRoute =
            serde_json::from_value(value).map_err(|e| error(e.to_string()))?;
        if route.executor_id != self.target.executor_id
            || route.generation != self.target.generation
            || route.profile_id != self.target.profile_id
            || route.expires_at_ms <= crate::now_ms()
        {
            return Err(error("stale executor route".into()));
        }
        let url =
            reqwest::Url::parse(&route.ws_url).map_err(|_| error("invalid route URL".into()))?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if url.scheme() != "wss" && !(url.scheme() == "ws" && loopback) {
            return Err(error("executor route requires WSS".into()));
        }
        let mut request = route
            .ws_url
            .into_client_request()
            .map_err(|_| error("invalid route URL".into()))?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", route.ticket)
                .parse()
                .map_err(|_| error("invalid route ticket".into()))?,
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|_| error("executor WSS connection failed".into()))?;
        Ok(Box::new(agent_daemon_byte_stream(socket)))
    }

    async fn session_token(&self) -> Result<String, String> {
        Ok(self.token.clone())
    }
    fn executor_target(&self) -> Option<ExecutorTarget> {
        Some(self.target.clone())
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutorListArgs {}

pub struct ExecutorListTool {
    directory: Arc<ExecutorDirectory>,
}

#[async_trait]
impl TypedTool for ExecutorListTool {
    type Args = ExecutorListArgs;
    type Output = Value;
    fn name(&self) -> &str {
        "executor_list"
    }
    fn description(&self) -> &str {
        "List authorized online executors with their current generation, profiles, ledger and actual capabilities. Offline devices are unavailable."
    }
    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        _args: Self::Args,
    ) -> Result<Value, AgentToolError> {
        Ok(json!({"executors": self.directory.list().await.map_err(AgentToolError::ExecFailed)?}))
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorAction {
    Describe {
        workdir: Option<String>,
    },
    Exec {
        cmd: String,
        cwd: String,
        #[serde(default)]
        env: BTreeMap<String, String>,
        timeout_ms: u64,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
        content: String,
    },
    Edit {
        path: String,
        old_string: String,
        new_string: String,
    },
    Query {
        operation_call_id: String,
        channel: Option<String>,
        #[serde(default)]
        offset: u64,
        #[serde(default)]
        wait_ms: u64,
    },
    Cancel {
        operation_call_id: String,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutorCallArgs {
    pub target: ExecutorTarget,
    pub action: ExecutorAction,
}

pub struct ExecutorCallTool {
    directory: Arc<ExecutorDirectory>,
    run: RunBindingSlot,
}

#[async_trait]
impl TypedTool for ExecutorCallTool {
    type Args = ExecutorCallArgs;
    type Output = Value;
    fn name(&self) -> &str {
        "executor_call"
    }
    fn description(&self) -> &str {
        "Execute a command or file operation on an explicit online executor without moving this Agent session. Persist operation locators before dispatch. Query/cancel use operation_call_id from the result; after reconnection select the new generation and retain the original store_id. Unknown outcomes are never replayed. Linux UI is unsupported."
    }
    async fn execute(&self, _ctx: &ToolCtx<'_>, args: Self::Args) -> Result<Value, AgentToolError> {
        let fail = |e: buckyos_api::ExecutorError| AgentToolError::Transport {
            message: e.message,
            effect_unknown: e.effect_unknown,
        };
        let binding = self.run.lock().unwrap().clone().ok_or_else(|| {
            AgentToolError::ExecFailed("Executor tools require a bound persistent run".into())
        })?;
        let directory = binding.run_dir.as_ref().ok_or_else(|| {
            AgentToolError::ExecFailed("Executor tools require a persistent run directory".into())
        })?;
        let connector = self
            .directory
            .connector(args.target.clone())
            .await
            .map_err(AgentToolError::ExecFailed)?;
        let client = NodeClient::new(
            connector,
            &args.target.executor_id,
            &args.target.profile_id,
            "executor_tool",
            self.run.clone(),
            &binding.run_id,
            Some(args.target.store_id.clone()),
        );
        if let ExecutorAction::Describe { workdir } = args.action {
            return Ok(json!(client.describe(workdir).await.map_err(fail)?));
        }
        let old_call = match &args.action {
            ExecutorAction::Query {
                operation_call_id, ..
            }
            | ExecutorAction::Cancel { operation_call_id } => Some(operation_call_id),
            _ => None,
        };
        if let Some(call_id) = old_call {
            let locator = read_locator_checked(directory, call_id)?.ok_or_else(|| {
                AgentToolError::InvalidArgs("Operation locator not found in this run".into())
            })?;
            if locator
                .executor_target
                .as_ref()
                .map(|target| &target.executor_id)
                != Some(&args.target.executor_id)
                || locator.profile_id != args.target.profile_id
                || locator.store_id != args.target.store_id
            {
                return Err(AgentToolError::InvalidArgs(
                    "Operation belongs to another executor, profile or ledger".into(),
                ));
            }
            return match args.action {
                ExecutorAction::Cancel { .. } => Ok(
                    json!({"locator": locator, "status": client.cancel(&locator.op_id).await.map_err(fail)?}),
                ),
                ExecutorAction::Query {
                    channel,
                    offset,
                    wait_ms,
                    ..
                } => {
                    let channel = match channel.as_deref() {
                        None => None,
                        Some("stdout") => Some(ExecutorChannel::Stdout),
                        Some("stderr") => Some(ExecutorChannel::Stderr),
                        Some("content") => Some(ExecutorChannel::Content),
                        _ => {
                            return Err(AgentToolError::InvalidArgs(
                                "Invalid result channel".into(),
                            ))
                        }
                    };
                    let (status, chunk) = client
                        .query(&locator.op_id, channel, offset, wait_ms.min(30_000))
                        .await
                        .map_err(fail)?;
                    Ok(
                        json!({"locator": locator, "status": status, "content": chunk.map(|(bytes, eof)| json!({"text": String::from_utf8_lossy(&bytes), "eof": eof}))}),
                    )
                }
                _ => unreachable!(),
            };
        }
        let call_id = super::CURRENT_TOOL_CALL
            .try_with(Clone::clone)
            .map_err(|_| {
                AgentToolError::ExecFailed("Executor tools require a tool call id".into())
            })?;
        if let Some(locator) = read_locator_checked(directory, &call_id)? {
            if locator
                .executor_target
                .as_ref()
                .map(|target| &target.executor_id)
                != Some(&args.target.executor_id)
                || locator.profile_id != args.target.profile_id
                || locator.store_id != args.target.store_id
            {
                return Err(AgentToolError::InvalidArgs(
                    "Existing call belongs to another target".into(),
                ));
            }
            let (status, _) = client
                .query(&locator.op_id, None, 0, 0)
                .await
                .map_err(fail)?;
            return Ok(json!({"locator": locator, "status": status, "operation_call_id": call_id}));
        }
        let op = match &args.action {
            ExecutorAction::Exec {
                cmd,
                cwd,
                env,
                timeout_ms,
            } => RemoteOp::Exec {
                cmd,
                cwd,
                env,
                timeout_ms: *timeout_ms,
            },
            ExecutorAction::Read { path } => RemoteOp::Read { path },
            ExecutorAction::Write { path, content } => RemoteOp::Write {
                path,
                mode: ExecutorWriteMode::Replace,
                content: content.as_bytes(),
            },
            ExecutorAction::Edit {
                path,
                old_string,
                new_string,
            } => RemoteOp::Edit {
                path,
                old: old_string,
                new: new_string,
            },
            _ => unreachable!(),
        };
        let (locator, status) = client.start_op(&call_id, op).await?;
        Ok(json!({"locator": locator, "status": status, "operation_call_id": call_id}))
    }
}

pub fn executor_tools(run: RunBindingSlot) -> Vec<Arc<dyn AgentTool>> {
    let directory = ExecutorDirectory::from_buckyos_runtime();
    vec![
        Arc::new(TypedToolHandle::with_null_host(ExecutorListTool {
            directory: directory.clone(),
        })),
        Arc::new(TypedToolHandle::with_null_host(ExecutorCallTool {
            directory,
            run,
        })),
    ]
}
