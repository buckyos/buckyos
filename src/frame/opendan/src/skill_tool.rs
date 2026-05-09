use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::sync::Mutex;

use crate::agent_session::{AgentSession, AgentSessionMgr, SessionSkillScope};
use crate::agent_tool::{AgentToolError, CallingConventions, ToolCtx, TypedTool};
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

#[derive(Deserialize)]
pub struct LoadSkillArgs {
    pub skill: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Serialize)]
pub struct LoadSkillOutput {
    ok: bool,
    scope: &'static str,
    skill: String,
    name: String,
    path: String,
}

#[async_trait]
impl TypedTool for LoadSkillTool {
    type Args = LoadSkillArgs;
    type Output = LoadSkillOutput;

    fn name(&self) -> &str {
        TOOL_LOAD_SKILL
    }

    fn description(&self) -> &str {
        "Load a prompt skill into the current behavior or the whole session."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ACTION | CallingConventions::LLM
    }

    fn args_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string" },
                "scope": { "type": "string", "enum": ["behavior", "session"] }
            },
            "required": ["skill"],
            "additionalProperties": false
        })
    }

    fn output_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "ok": { "type": "boolean" },
                "scope": { "type": "string" },
                "skill": { "type": "string" },
                "path": { "type": "string" }
            }
        })
    }

    fn usage(&self) -> Option<String> {
        Some("load_skill <skill> [behavior|session]".to_string())
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let scope = args
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("behavior");
        Some(format!("{TOOL_LOAD_SKILL} {} {}", args.skill.trim(), scope))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("loaded skill `{}` into {} scope", output.name, output.scope)
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let skill_ref = args.skill.trim();
        if skill_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing or invalid `skill`".to_string(),
            ));
        }
        let scope = parse_skill_scope(args.scope.as_deref())?;

        let session_id = ctx.session().session_id.as_str();
        self.session_store
            .refresh_status_from_disk(session_id)
            .await?;
        let session = self
            .session_store
            .get_session(session_id)
            .await
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!("session not found: {}", session_id))
            })?;

        let resolved = resolve_skill_for_session(session.clone(), skill_ref).await?;
        {
            let mut guard = session.lock().await;
            guard.load_skill_ref(resolved.reference.as_str(), scope);
            self.session_store.save_session_locked(&guard).await?;
        }

        Ok(LoadSkillOutput {
            ok: true,
            scope: render_scope(scope),
            skill: resolved.reference,
            name: resolved.name,
            path: resolved.path,
        })
    }
}

#[derive(Deserialize)]
pub struct UnloadSkillArgs {
    pub skill: String,
}

#[derive(Serialize)]
pub struct UnloadSkillOutput {
    ok: bool,
    skill: String,
}

#[async_trait]
impl TypedTool for UnloadSkillTool {
    type Args = UnloadSkillArgs;
    type Output = UnloadSkillOutput;

    fn name(&self) -> &str {
        TOOL_UNLOAD_SKILL
    }

    fn description(&self) -> &str {
        "Unload a previously loaded prompt skill from the current session."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ACTION | CallingConventions::LLM
    }

    fn args_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string" }
            },
            "required": ["skill"],
            "additionalProperties": false
        })
    }

    fn output_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "ok": { "type": "boolean" },
                "skill": { "type": "string" }
            }
        })
    }

    fn usage(&self) -> Option<String> {
        Some("unload_skill <skill>".to_string())
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        Some(format!("{TOOL_UNLOAD_SKILL} {}", args.skill.trim()))
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("unloaded skill `{}`", output.skill)
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let skill_ref = args.skill.trim().to_string();
        if skill_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing or invalid `skill`".to_string(),
            ));
        }
        let session_id = ctx.session().session_id.as_str();
        self.session_store
            .refresh_status_from_disk(session_id)
            .await?;
        let session = self
            .session_store
            .get_session(session_id)
            .await
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!("session not found: {}", session_id))
            })?;

        {
            let mut guard = session.lock().await;
            guard.unload_skill_ref(skill_ref.as_str());
            self.session_store.save_session_locked(&guard).await?;
        }

        Ok(UnloadSkillOutput {
            ok: true,
            skill: skill_ref,
        })
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
    use crate::agent_tool::NullToolHost;
    use crate::behavior::SessionRuntimeContext;
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
        let ctx = SessionRuntimeContext {
            trace_id: "t1".to_string(),
            agent_name: "did:test:agent".to_string(),
            behavior: "plan".to_string(),
            step_idx: 0,
            wakeup_id: "w1".to_string(),
            session_id: "s1".to_string(),
        };
        let host = NullToolHost;
        let tool_ctx = ToolCtx::new(&ctx, &host);
        tool.execute(
            &tool_ctx,
            LoadSkillArgs {
                skill: "planner".to_string(),
                scope: Some("session".to_string()),
            },
        )
        .await
        .expect("load skill");

        let session = store.get_session("s1").await.expect("get session");
        let guard = session.lock().await;
        assert_eq!(guard.session_loaded_skills, vec!["planner".to_string()]);
    }
}
