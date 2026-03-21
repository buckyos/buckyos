use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, warn};
use tokio::sync::RwLock;

pub use ::agent_tool::*;

use crate::behavior::{BehaviorConfig, BehaviorExecInput, PolicyEngine};
pub use crate::buildin_tool::{TOOL_EDIT_FILE, TOOL_EXEC_BASH, TOOL_READ_FILE, TOOL_WRITE_FILE};

pub const TOOL_CREATE_SUB_AGENT: &str = "create_sub_agent";

#[derive(Clone)]
pub struct AgentPolicy {
    tool_mgr: Arc<AgentToolManager>,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
}

impl AgentPolicy {
    pub fn new(
        tool_mgr: Arc<AgentToolManager>,
        behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    ) -> Self {
        Self {
            tool_mgr,
            behavior_cfg_cache,
        }
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn allowed_tools(&self, input: &BehaviorExecInput) -> Result<Vec<ToolSpec>, String> {
        let all = self.tool_mgr.list_tool_specs();
        let cfg = {
            let guard = self.behavior_cfg_cache.read().await;
            guard.get(&input.trace.behavior).cloned()
        };
        if let Some(cfg) = cfg {
            let filtered = cfg.tools.filter_tool_specs(&all);
            debug!(
                "ai_agent.policy allowed_tools: behavior={} all={} filtered={}",
                input.trace.behavior,
                all.len(),
                filtered.len()
            );
            Ok(filtered)
        } else {
            debug!(
                "ai_agent.policy allowed_tools: behavior={} all={} filtered={} default_allow_all=true",
                input.trace.behavior,
                all.len(),
                all.len()
            );
            Ok(all)
        }
    }

    async fn gate_tool_calls(
        &self,
        input: &BehaviorExecInput,
        calls: &[buckyos_api::AiToolCall],
    ) -> Result<Vec<buckyos_api::AiToolCall>, String> {
        let allowed = self.allowed_tools(input).await?;
        let allowed_names = allowed
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::HashSet<_>>();
        let mut gated = Vec::with_capacity(calls.len());
        for call in calls {
            if !allowed_names.contains(&call.name) {
                warn!(
                    "ai_agent.policy deny_tool_call: behavior={} tool={} calls={}",
                    input.trace.behavior,
                    call.name,
                    calls.len()
                );
                return Err(format!(
                    "tool `{}` is not allowed for behavior `{}`",
                    call.name, input.trace.behavior
                ));
            }
            gated.push(call.clone());
        }
        Ok(gated)
    }
}
