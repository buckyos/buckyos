use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value as Json};
use tokio::sync::Mutex;

use crate::agent_session::{AgentSession, AgentSessionMgr, SessionSkillScope};
use crate::agent_tool::{
    optional_trimmed_string_arg, require_trimmed_string_arg, AgentTool, AgentToolError,
    AgentToolResult, ToolSpec,
};
use crate::behavior::SessionRuntimeContext;
use crate::workspace::agent_skill::{self, AgentSkillSpec};
use crate::workspace_path::{
    resolve_agent_env_root, resolve_agent_env_root_from_local_workspace_hint,
    resolve_default_local_workspace_path,
};

pub const TOOL_LOAD_SKILL: &str = "load_skill";
pub const TOOL_UNLOAD_SKILL: &str = "unload_skill";

#[derive(Clone)]
pub struct LoadSkillTool {
    session_store: Arc<AgentSessionMgr>,
}

impl LoadSkillTool {
    pub fn new(session_store: Arc<AgentSessionMgr>) -> Self {
        Self { session_store }
    }
}

#[derive(Clone)]
pub struct UnloadSkillTool {
    session_store: Arc<AgentSessionMgr>,
}

impl UnloadSkillTool {
    pub fn new(session_store: Arc<AgentSessionMgr>) -> Self {
        Self { session_store }
    }
}

#[async_trait]
impl AgentTool for LoadSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LOAD_SKILL.to_string(),
            description: "Load a prompt skill into the current behavior or the whole session."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string" },
                    "scope": { "type": "string", "enum": ["behavior", "session"] }
                },
                "required": ["skill"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "scope": { "type": "string" },
                    "skill": { "type": "string" },
                    "path": { "type": "string" }
                }
            }),
            usage: Some("load_skill <skill> [behavior|session]".to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        false
    }

    fn support_action(&self) -> bool {
        true
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let skill_ref = require_trimmed_string_arg(&args, "skill")?;
        let scope = parse_skill_scope(optional_trimmed_string_arg(&args, "scope")?.as_deref())?;

        self.session_store
            .refresh_status_from_disk(ctx.session_id.as_str())
            .await?;
        let session = self
            .session_store
            .get_session(ctx.session_id.as_str())
            .await
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!("session not found: {}", ctx.session_id))
            })?;

        let resolved = resolve_skill_for_session(session.clone(), skill_ref.as_str()).await?;
        {
            let mut guard = session.lock().await;
            guard.load_skill_ref(resolved.reference.as_str(), scope);
            self.session_store.save_session_locked(&guard).await?;
        }

        Ok(AgentToolResult::from_details(json!({
            "ok": true,
            "scope": render_scope(scope),
            "skill": resolved.reference,
            "name": resolved.name,
            "path": resolved.path,
        }))
        .with_is_agent_tool(true)
        .with_cmd_line(format!("load_skill {} {}", skill_ref, render_scope(scope)))
        .with_result(format!(
            "loaded skill `{}` into {} scope",
            resolved.name,
            render_scope(scope)
        )))
    }
}

#[async_trait]
impl AgentTool for UnloadSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_UNLOAD_SKILL.to_string(),
            description: "Unload a previously loaded prompt skill from the current session."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string" }
                },
                "required": ["skill"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "skill": { "type": "string" }
                }
            }),
            usage: Some("unload_skill <skill>".to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        false
    }

    fn support_action(&self) -> bool {
        true
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let skill_ref = require_trimmed_string_arg(&args, "skill")?;
        self.session_store
            .refresh_status_from_disk(ctx.session_id.as_str())
            .await?;
        let session = self
            .session_store
            .get_session(ctx.session_id.as_str())
            .await
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!("session not found: {}", ctx.session_id))
            })?;

        {
            let mut guard = session.lock().await;
            guard.unload_skill_ref(skill_ref.as_str());
            self.session_store.save_session_locked(&guard).await?;
        }

        Ok(AgentToolResult::from_details(json!({
            "ok": true,
            "skill": skill_ref,
        }))
        .with_is_agent_tool(true)
        .with_cmd_line(format!("unload_skill {}", skill_ref))
        .with_result(format!("unloaded skill `{}`", skill_ref)))
    }
}

fn parse_skill_scope(raw: Option<&str>) -> Result<SessionSkillScope, AgentToolError> {
    match raw
        .unwrap_or("behavior")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "behavior" => Ok(SessionSkillScope::Behavior),
        "session" => Ok(SessionSkillScope::Session),
        other => Err(AgentToolError::InvalidArgs(format!(
            "unsupported skill scope `{other}`"
        ))),
    }
}

fn render_scope(scope: SessionSkillScope) -> &'static str {
    match scope {
        SessionSkillScope::Behavior => "behavior",
        SessionSkillScope::Session => "session",
    }
}

async fn resolve_skill_for_session(
    session: Arc<Mutex<AgentSession>>,
    skill_ref: &str,
) -> Result<AgentSkillSpec, AgentToolError> {
    let (cwd, local_workspace_id, workspace_info) = {
        let guard = session.lock().await;
        (
            guard.pwd.clone(),
            guard.local_workspace_id.clone(),
            guard.workspace_info.clone(),
        )
    };
    let roots =
        build_session_skill_roots(local_workspace_id.as_deref(), workspace_info.as_ref(), &cwd);
    agent_skill::load_skill_from_roots(&roots, &cwd, skill_ref).await
}

fn build_session_skill_roots(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Vec<PathBuf> {
    let mut roots = Vec::<PathBuf>::new();
    if let Some(agent_root) = resolve_agent_env_root(workspace_info, session_cwd).or_else(|| {
        resolve_agent_env_root_from_local_workspace_hint(
            local_workspace_id,
            workspace_info,
            session_cwd,
        )
    }) {
        roots.push(agent_root.join(agent_skill::SKILLS_REL_PATH));
    }
    if let Some(workspace_root) =
        resolve_default_local_workspace_path(local_workspace_id, workspace_info, session_cwd)
    {
        roots.push(workspace_root.join(agent_skill::SKILLS_REL_PATH));
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn load_skill_tool_persists_session_scope() {
        let temp = tempdir().expect("create temp dir");
        let skills_dir = temp.path().join("skills/planner");
        tokio::fs::create_dir_all(&skills_dir)
            .await
            .expect("create skill dir");
        tokio::fs::write(
            skills_dir.join("meta.json"),
            r#"{"name":"planner","summary":"planning helper"}"#,
        )
        .await
        .expect("write skill meta");
        tokio::fs::write(skills_dir.join("skill.md"), "plan carefully")
            .await
            .expect("write skill body");

        let store = Arc::new(
            AgentSessionMgr::new(
                "did:test:agent",
                temp.path(),
                "plan".to_string(),
                "plan".to_string(),
            )
            .await
            .expect("create session store"),
        );
        let session = store
            .ensure_session("s1", None, None, None)
            .await
            .expect("ensure session");
        session.lock().await.pwd = temp.path().to_path_buf();
        store.save_session("s1").await.expect("save session");

        let tool = LoadSkillTool::new(store.clone());
        tool.call(
            &SessionRuntimeContext {
                trace_id: "t1".to_string(),
                agent_name: "did:test:agent".to_string(),
                behavior: "plan".to_string(),
                step_idx: 0,
                wakeup_id: "w1".to_string(),
                session_id: "s1".to_string(),
            },
            json!({"skill": "planner", "scope": "session"}),
        )
        .await
        .expect("load skill");

        let session = store.get_session("s1").await.expect("get session");
        let guard = session.lock().await;
        assert_eq!(guard.session_loaded_skills, vec!["planner".to_string()]);
    }
}
