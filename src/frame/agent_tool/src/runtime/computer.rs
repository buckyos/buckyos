use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use buckyos_api::{ComputerAccess, ComputerRequest, ExecutorChannel, ExecutorDescribeOk};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::node::{BuckyosNodeConnector, NodeClient, NodeExecutorConnector, NodeIo, RemoteOp};
use crate::llm_bash::{new_run_binding_slot, RunBindingSlot};
use crate::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolCtx, ToolSpec, TypedTool, TypedToolHandle,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeComputerConfig {
    pub endpoint: String,
    pub node_did: String,
    pub profile_id: String,
    pub access: ComputerAccess,
}

struct LocalConnector {
    endpoint: String,
    node_did: String,
    tokens: Arc<dyn NodeExecutorConnector>,
}

#[async_trait]
impl NodeExecutorConnector for LocalConnector {
    async fn connect(&self, target: &str) -> std::io::Result<Box<dyn NodeIo>> {
        if target != self.node_did {
            return Err(std::io::Error::other("local computer endpoint mismatch"));
        }
        buckyos_api::node_executor_ipc::connect_local_executor(&self.endpoint).await
    }

    async fn session_token(&self) -> Result<String, String> {
        self.tokens.session_token().await
    }
}

fn client(
    config: &NativeComputerConfig,
    run: RunBindingSlot,
    run_id: &str,
    tokens: Arc<dyn NodeExecutorConnector>,
) -> NodeClient {
    NodeClient::new(
        Arc::new(LocalConnector {
            endpoint: config.endpoint.clone(),
            node_did: config.node_did.clone(),
            tokens,
        }),
        &config.node_did,
        &config.profile_id,
        "private",
        run,
        run_id,
        None,
    )
}

pub async fn describe(config: &NativeComputerConfig) -> Result<ExecutorDescribeOk, String> {
    tokio::time::timeout(
        Duration::from_secs(3),
        client(
            config,
            new_run_binding_slot(),
            "computer-check",
            Arc::new(BuckyosNodeConnector),
        )
        .describe(None),
    )
    .await
    .map_err(|_| "local computer IPC check timed out".to_string())?
    .map_err(|e| format!("local computer IPC: {}", e.message))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ComputerArgs {
    #[schemars(with = "ComputerRequest")]
    request: Value,
}

struct ComputerTool {
    config: NativeComputerConfig,
    run: RunBindingSlot,
    tokens: Arc<dyn NodeExecutorConnector>,
}

#[async_trait]
impl TypedTool for ComputerTool {
    type Args = ComputerArgs;
    type Output = Value;

    fn name(&self) -> &str {
        "computer"
    }

    fn description(&self) -> &str {
        "Use this native runtime's desktop through local IPC. request uses the Computer API: command inspect, observe, click, type, keypress, scroll or wait_for; binding_id is filled from the local display when empty or omitted. Start with inspect or observe (mode: visual or auto); inspect reports the modes supported by the local driver. Results are JSON: status.result.computer contains execution, error and observation; screenshot is an absolute local PNG path. Use llm_understand_media on that local file to locate a target in the observation's original screenshot width and height. Actions must use that observation_id, binding_id and screenshot coordinates before expires_at_ms; if the observation expires or the screen changes, observe again and verify the target. An unknown execution outcome must not be blindly retried. Owner permissions, focus checks and desktop locking are enforced by the existing driver."
    }

    async fn execute(&self, _ctx: &ToolCtx<'_>, args: Self::Args) -> Result<Value, AgentToolError> {
        match (
            self.config.access,
            args.request.get("command").and_then(Value::as_str),
        ) {
            (ComputerAccess::None, _) => {
                return Err(AgentToolError::InvalidArgs(
                    "computer access is disabled".into(),
                ))
            }
            (ComputerAccess::Observe, Some("inspect" | "observe"))
            | (ComputerAccess::Control, _) => {}
            _ => {
                return Err(AgentToolError::InvalidArgs(
                    "computer action requires control permission".into(),
                ))
            }
        }
        let fail = |e: buckyos_api::ExecutorError| AgentToolError::Transport {
            message: format!("local computer IPC: {}", e.message),
            effect_unknown: e.effect_unknown,
        };
        let binding = self
            .run
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AgentToolError::ExecFailed("computer needs a bound run".into()))?;
        let run_dir = binding.run_dir.as_ref().ok_or_else(|| {
            AgentToolError::ExecFailed("computer needs a persistent run directory".into())
        })?;
        let client = client(
            &self.config,
            self.run.clone(),
            &binding.run_id,
            self.tokens.clone(),
        );
        let mut request = args.request;
        if request
            .get("binding_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            let described = client.describe(None).await.map_err(fail)?;
            let display = described
                .info
                .desktop
                .as_ref()
                .and_then(|d| d.pointer("/ui/display_id"))
                .and_then(Value::as_u64)
                .ok_or_else(|| AgentToolError::ExecFailed("local desktop has no display".into()))?;
            request["binding_id"] = json!(format!("{}:display:{display}", self.config.profile_id));
        }
        let request: ComputerRequest = serde_json::from_value(request)
            .map_err(|e| AgentToolError::InvalidArgs(e.to_string()))?;
        request
            .validate()
            .map_err(|e| AgentToolError::InvalidArgs(e.to_string()))?;
        let permitted = match self.config.access {
            ComputerAccess::None => false,
            ComputerAccess::Observe => matches!(
                request,
                ComputerRequest::Inspect { .. } | ComputerRequest::Observe { .. }
            ),
            ComputerAccess::Control => true,
        };
        if !permitted {
            return Err(AgentToolError::InvalidArgs(
                "computer operation is outside the owner's grant".into(),
            ));
        }
        let call_id = super::CURRENT_TOOL_CALL
            .try_with(Clone::clone)
            .map_err(|_| AgentToolError::ExecFailed("computer needs a tool call id".into()))?;
        let (locator, initial) = client
            .start_op(&call_id, RemoteOp::Computer { request: &request })
            .await?;
        let status = if initial.state.is_terminal() {
            initial
        } else {
            client
                .wait_terminal(
                    &locator.op_id,
                    Instant::now() + Duration::from_millis(request.execution_timeout_ms() + 5_000),
                )
                .await
                .map_err(fail)?
                .ok_or_else(|| AgentToolError::Transport {
                    message: format!(
                        "local computer operation {} has not finished; its outcome is unknown",
                        locator.op_id
                    ),
                    effect_unknown: true,
                })?
        };
        let has_image = status
            .result
            .as_ref()
            .and_then(|r| r.content.as_ref())
            .is_some_and(|c| c.retained_bytes > 0);
        let mut result = json!({"status": status, "operation_id": locator.op_id});
        if has_image {
            let image = async {
                let png = client
                    .read_channel(
                        &locator.op_id,
                        ExecutorChannel::Content,
                        buckyos_api::COMPUTER_MAX_IMAGE_BYTES,
                    )
                    .await
                    .map_err(|e| e.message)?;
                let dir = run_dir.join("computer");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!(
                    "{}.png",
                    crate::llm_bash::sanitize_call_id(&call_id)
                ));
                std::fs::write(&path, png).map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            }
            .await;
            match image {
                Ok(path) => result["screenshot"] = json!(path),
                Err(error) => result["screenshot_error"] = json!(error),
            }
        }
        Ok(result)
    }
}

struct ComputerToolHandle(TypedToolHandle<ComputerTool>);

#[async_trait]
impl AgentTool for ComputerToolHandle {
    fn spec(&self) -> ToolSpec {
        self.0.spec()
    }

    fn calling(&self) -> CallingConventions {
        self.0.calling()
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let result = self.0.call(ctx, args).await?;
        let state = result
            .details
            .pointer("/status/state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status = if state == "succeeded" {
            AgentToolStatus::Success
        } else {
            AgentToolStatus::Error
        };
        let summary = match result
            .details
            .pointer("/status/result/computer/error/message")
            .and_then(Value::as_str)
        {
            Some(error) => format!("computer {state}: {error}"),
            None => format!("computer {state}"),
        };
        let output = result.details.to_string();
        Ok(result
            .with_output(output)
            .with_result(summary)
            .with_status(status)
            .refresh_default_title())
    }
}

pub fn tool(
    config: NativeComputerConfig,
    run: RunBindingSlot,
    tokens: Option<Arc<dyn NodeExecutorConnector>>,
) -> Arc<dyn AgentTool> {
    Arc::new(ComputerToolHandle(TypedToolHandle::new(ComputerTool {
        config,
        run,
        tokens: tokens.unwrap_or_else(|| Arc::new(BuckyosNodeConnector)),
    })))
}

#[cfg(test)]
#[path = "computer_result_tests.rs"]
mod result_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::*;
    use tokio::io::AsyncWriteExt;

    struct TokensOnly;
    #[async_trait]
    impl NodeExecutorConnector for TokensOnly {
        async fn connect(&self, _: &str) -> std::io::Result<Box<dyn NodeIo>> {
            panic!("native computer must never use a network connector")
        }
        async fn session_token(&self) -> Result<String, String> {
            Ok("agent-token".into())
        }
    }

    #[tokio::test]
    async fn native_computer_uses_ipc_and_checks_the_real_node_identity() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = if cfg!(windows) {
            node_executor_ipc::local_executor_endpoint(dir.path())
        } else {
            dir.path().join("computer.sock").display().to_string()
        };
        let mut listener = node_executor_ipc::LocalExecutorListener::bind(&endpoint)
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let Some(ExecutorFrame::Control(login)) =
                read_executor_frame(&mut stream).await.unwrap()
            else {
                panic!()
            };
            let ExecutorClientMessage::Login(login) = serde_json::from_slice(&login).unwrap()
            else {
                panic!()
            };
            assert_eq!(login.token, "agent-token");
            let reply = json!({"type":"login_ok", "request_id":"login", "posture":"normal", "principal":"agent", "app_instance_id":"agent@owner",
                "caps":["describe", "computer_observe"], "limits":ExecutorLimits::default(), "revision":"one", "generation":1,
                "auth_valid_until_ms":crate::now_ms()+60_000, "token_expires_at_ms":crate::now_ms()+60_000, "store_id":"ledger", "server_epoch":"epoch"});
            stream
                .write_all(&encode_executor_control(&reply).unwrap())
                .await
                .unwrap();
            let Some(ExecutorFrame::Control(request)) =
                read_executor_frame(&mut stream).await.unwrap()
            else {
                panic!()
            };
            let ExecutorClientMessage::Describe(request) =
                serde_json::from_slice(&request).unwrap()
            else {
                panic!()
            };
            let reply = json!({"type":"describe_ok", "request_id":request.request_id,
                "target":{"node_did":"did:web:ood1", "profile_id":"agent-test", "workdir_roots":[], "store_id":"ledger", "protocol_version":NODE_EXECUTOR_PROTOCOL_VERSION},
                "info":{"os":"macos", "arch":"aarch64", "hostname":"desktop", "shell":"bash", "time_ms":crate::now_ms(),
                    "desktop":{"ui":{"supported":true,"ready":true,"display_id":1}}}});
            stream
                .write_all(&encode_executor_control(&reply).unwrap())
                .await
                .unwrap();
        });
        let config = NativeComputerConfig {
            endpoint,
            node_did: "did:web:ood1".into(),
            profile_id: "agent-test".into(),
            access: ComputerAccess::Observe,
        };
        let described = client(&config, new_run_binding_slot(), "run", Arc::new(TokensOnly))
            .describe(None)
            .await
            .unwrap();
        assert_eq!(described.target.node_did, config.node_did);
        assert_eq!(described.info.desktop.unwrap()["ui"]["ready"], true);
        server.await.unwrap();
        let runtime = super::super::NativeRuntime::from_config(super::super::RuntimeConfig {
            computer: Some(config),
            workdir: Some(dir.path().display().to_string()),
            ..Default::default()
        });
        use super::super::AgentRuntime;
        let ctx = super::super::RuntimeOpenCtx::new(
            dir.path(),
            "run",
            crate::xllm::LoopModel::FunctionCall,
        );
        let (tools, manager) = runtime
            .open(
                &ctx,
                &crate::xllm::ToolsConfig {
                    enabled: Some(true),
                    ..Default::default()
                },
                &Default::default(),
            )
            .await
            .unwrap();
        assert!(tools.all_names().iter().any(|name| name == "computer"));
        assert!(manager.has("shell"));
        assert_eq!(runtime.descriptor().kind, "native");
    }
}
