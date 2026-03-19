use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::fs;

use crate::agent_tool::AgentToolError;

pub const SKILLS_REL_PATH: &str = "skills";
const SKILL_FILE_EXTENSIONS: [&str; 3] = ["yaml", "yml", "json"];

#[derive(Clone, Debug)]
pub struct AgentSkillRecord {
    pub name: String,
    pub introduce: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct AgentSkillSpec {
    pub name: String,
    pub introduce: String,
    pub rules: String,
    //先不支持自定义action,只能引用runtime里已经定义好的Action
    pub actions: Vec<String>,
    //先不支持自定义tool,只能引用runtime里已经定义好的tool
    pub loaded_tools: Vec<String>,
}

impl AgentSkillSpec {
    fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.introduce = self.introduce.trim().to_string();
        self.rules = self.rules.trim().to_string();
        self.actions = normalize_unique_string_list(std::mem::take(&mut self.actions));
        self.loaded_tools = normalize_unique_string_list(std::mem::take(&mut self.loaded_tools));
    }
}

pub async fn merge_skill_records_from_dir(
    skills_root: &Path,
    records: &mut HashMap<String, AgentSkillRecord>,
) -> Result<(), AgentToolError> {
    if !fs::try_exists(skills_root)
        .await
        .map_err(|err| io_error("check skills dir", skills_root, err))?
    {
        return Ok(());
    }

    let mut entries = fs::read_dir(skills_root)
        .await
        .map_err(|err| io_error("read skills dir", skills_root, err))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|err| io_error("read skill dir entry", skills_root, err))?
    {
        let entry_path = entry.path();
        let metadata = entry
            .metadata()
            .await
            .map_err(|err| io_error("read skill dir metadata", &entry_path, err))?;
        if !metadata.is_dir() {
            continue;
        }

        let Some(skill_key_raw) = entry.file_name().to_str().map(|value| value.to_string()) else {
            continue;
        };
        let skill_key = skill_key_raw.trim();
        if skill_key.is_empty() {
            continue;
        }

        let Some(skill_file_path) = find_skill_file(skills_root, skill_key).await? else {
            continue;
        };
        let raw_spec = read_skill_spec_file(&skill_file_path).await?;
        let skill_name = if raw_spec.name.is_empty() {
            skill_key.to_string()
        } else {
            raw_spec.name.clone()
        };

        records.insert(
            skill_key.to_string(),
            AgentSkillRecord {
                name: skill_name,
                introduce: raw_spec.introduce,
            },
        );
    }

    Ok(())
}

pub async fn load_skill_from_root(
    skills_root: &Path,
    skill_name: &str,
) -> Result<AgentSkillSpec, AgentToolError> {
    let skill_name = validate_skill_name(skill_name)?;
    let skill_path = find_skill_file(skills_root, skill_name)
        .await?
        .ok_or_else(|| AgentToolError::NotFound(format!("skill not found: {skill_name}")))?;
    read_skill_spec_file(&skill_path).await
}

async fn find_skill_file(
    skills_root: &Path,
    skill_name: &str,
) -> Result<Option<PathBuf>, AgentToolError> {
    let skill_name = skill_name.trim();
    if skill_name.is_empty() {
        return Ok(None);
    }

    let skill_dir = skills_root.join(skill_name);
    if !fs::try_exists(&skill_dir)
        .await
        .map_err(|err| io_error("check skill dir", &skill_dir, err))?
    {
        return Ok(None);
    }

    let metadata = fs::metadata(&skill_dir)
        .await
        .map_err(|err| io_error("read skill dir metadata", &skill_dir, err))?;
    if !metadata.is_dir() {
        return Ok(None);
    }

    for ext in SKILL_FILE_EXTENSIONS {
        let path = skill_dir.join(format!("{skill_name}.{ext}"));
        if fs::try_exists(&path)
            .await
            .map_err(|err| io_error("check skill file", &path, err))?
        {
            let file_meta = fs::metadata(&path)
                .await
                .map_err(|err| io_error("read skill file metadata", &path, err))?;
            if file_meta.is_file() {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

async fn read_skill_spec_file(path: &Path) -> Result<AgentSkillSpec, AgentToolError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|err| io_error("read skill file", path, err))?;

    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    let mut spec =
        match ext.as_deref() {
            Some("json") => {
                serde_json::from_str::<AgentSkillSpec>(&content).map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "parse skill config `{}` failed: {err}",
                        path.display()
                    ))
                })?
            }
            Some("yaml") | Some("yml") => {
                serde_yaml::from_str::<AgentSkillSpec>(&content).map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "parse skill config `{}` failed: {err}",
                        path.display()
                    ))
                })?
            }
            _ => serde_json::from_str::<AgentSkillSpec>(&content).or_else(|json_err| {
                serde_yaml::from_str::<AgentSkillSpec>(&content).map_err(|yaml_err| {
                    AgentToolError::ExecFailed(format!(
                        "parse skill config `{}` failed: json error: {json_err}; yaml error: {yaml_err}",
                        path.display()
                    ))
                })
            })?,
        };

    spec.normalize();
    Ok(spec)
}

fn validate_skill_name(input: &str) -> Result<&str, AgentToolError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "skill_name cannot be empty".to_string(),
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(AgentToolError::InvalidArgs(
            "skill_name cannot contain path separators".to_string(),
        ));
    }
    if trimmed.contains("..") {
        return Err(AgentToolError::InvalidArgs(
            "skill_name cannot contain `..`".to_string(),
        ));
    }
    Ok(trimmed)
}

fn normalize_unique_string_list(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn io_error(action: &str, path: &Path, source: std::io::Error) -> AgentToolError {
    AgentToolError::ExecFailed(format!("{action} `{}` failed: {source}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use buckyos_api::{value_to_object_map, AiToolSpec};
    use tokio::sync::Mutex;

    use crate::agent_environment::AgentEnvironment;
    use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
    use crate::agent_session::{AgentSession, AgentSessionMgr, GetSessionTool};
    use crate::agent_tool::{AgentToolManager, ToolSpec};
    use crate::behavior::{
        BehaviorConfig, BehaviorExecInput, PromptBuilder, SessionRuntimeContext, StepLimits,
        Tokenizer,
    };

    struct MockTokenizer;

    impl Tokenizer for MockTokenizer {
        fn count_tokens(&self, text: &str) -> u32 {
            text.split_whitespace().count() as u32
        }
    }

    async fn load_runtime_tool_specs(agent_env_root: &Path) -> (Vec<AiToolSpec>, Vec<ToolSpec>) {
        let tool_mgr = AgentToolManager::new();
        let session_store = Arc::new(
            AgentSessionMgr::new(
                "did:web:agent.example.com",
                agent_env_root.join("sessions"),
                "resolve_router".to_string(),
            )
            .await
            .expect("create session store"),
        );

        let environment = AgentEnvironment::new(agent_env_root.to_path_buf())
            .await
            .expect("create agent environment");
        environment
            .register_workshop_tools(&tool_mgr, session_store.clone())
            .expect("register workshop tools");

        let memory = AgentMemory::new(AgentMemoryConfig::new(agent_env_root.to_path_buf()))
            .await
            .expect("create memory");
        memory
            .register_tools(&tool_mgr)
            .expect("register memory tools");

        tool_mgr
            .register_tool(GetSessionTool::new(session_store))
            .expect("register get_session tool");

        let tools = tool_mgr
            .list_tool_specs()
            .into_iter()
            .map(|tool| AiToolSpec {
                name: tool.name.clone(),
                description: tool.description.clone(),
                args_schema: value_to_object_map(tool.args_schema.clone()),
                output_schema: tool.output_schema.clone(),
            })
            .collect::<Vec<_>>();
        let action_specs = tool_mgr.list_action_specs();
        (tools, action_specs)
    }

    async fn render_toolbox_prompt_preview(
        skill_name: &str,
        agent_env_root: &Path,
    ) -> (String, Vec<AiToolSpec>) {
        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = agent_env_root.to_path_buf();
        session.loaded_skills = vec![skill_name.to_string()];
        let session = Arc::new(Mutex::new(session));

        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-1".to_string(),
            },
            input_prompt: "preview toolbox prompt".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a test agent.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: String::new(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig::default(),
            session: None,
        };

        let (tools, action_specs) = load_runtime_tool_specs(agent_env_root).await;

        let req = PromptBuilder::build(
            &input,
            &tools,
            &action_specs,
            &input.behavior_cfg,
            &MockTokenizer,
            Some(session),
            None,
        )
        .await
        .expect("build prompt");
        let system_prompt = req
            .payload
            .messages
            .first()
            .map(|msg| msg.content.as_str())
            .unwrap_or_default();

        return (system_prompt.to_string(), req.payload.tool_specs);
    }

    #[tokio::test]
    async fn load_skill_from_root_prints_toolbox_prompt_preview() {
        let skills_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rootfs/bin/buckyos_jarvis/skills");
        let agent_env_root = skills_root.parent().unwrap();
        let skill_name = "chatonly";

        let spec = load_skill_from_root(&skills_root, skill_name)
            .await
            .expect("load skill");
        assert_eq!(spec.introduce, "no action skill, only chat with the user.");

        let (toolbox_prompt, tool_specs) =
            render_toolbox_prompt_preview(skill_name, agent_env_root).await;
        println!(
            "\n[toolbox section prompt preview after loading skill `{}`]\n{}\n",
            skill_name, toolbox_prompt
        );
        for tool_spec in tool_specs {
            println!("tool_spec: {:?}", tool_spec);
        }
        println!("--------------------------------");
        let skill_name = "planner";
        load_skill_from_root(&skills_root, skill_name)
            .await
            .expect("load skill");

        let (toolbox_prompt, tool_specs) =
            render_toolbox_prompt_preview(skill_name, agent_env_root).await;
        println!(
            "\n[toolbox section prompt preview after loading skill `{}`]\n{}\n",
            skill_name, toolbox_prompt
        );
        for tool_spec in tool_specs {
            println!("tool_spec: {:?}", tool_spec);
        }
    }
}
