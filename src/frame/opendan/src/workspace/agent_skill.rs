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
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::{json, Value as Json};

    fn unique_skills_root(test_name: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        std::env::temp_dir().join(format!("opendan-agent-skill-{test_name}-{ts}"))
    }

    fn render_toolbox_prompt_preview(skill_name: &str, spec: &AgentSkillSpec) -> String {
        let selected_actions = spec.actions.clone();
        let payload: Json = json!({
            "workspace_skill_records": [
                {
                    "name": skill_name,
                    "introduce": spec.introduce,
                }
            ],
            "loaded_skills": [skill_name],
            "requested_actions": spec.actions,
            "allow_actions": selected_actions,
            "actions": selected_actions,
            "unresolved_actions": [],
            "action_specs": [],
            "action_prompts": [],
            "loaded_tools_preview": spec.loaded_tools,
            "skill_rules_preview": spec.rules,
        });
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    }

    #[tokio::test]
    async fn load_skill_from_root_prints_toolbox_prompt_preview() {
        let root = unique_skills_root("toolbox-preview");
        let skill_name = "planner";
        let skill_dir = root.join(skill_name);
        tokio::fs::create_dir_all(&skill_dir)
            .await
            .expect("create skill dir");
        tokio::fs::write(
            skill_dir.join("planner.yaml"),
            "name: planner
introduce: Plan and track work items
rules: Keep tasks explicit and check dependencies
actions: [plan, execute, plan]
loaded_tools: [exec_bash, read_file, exec_bash]
",
        )
        .await
        .expect("write skill config");

        let spec = load_skill_from_root(&root, skill_name)
            .await
            .expect("load skill");
        assert_eq!(spec.introduce, "Plan and track work items");
        assert_eq!(spec.actions, vec!["plan".to_string(), "execute".to_string()]);
        assert_eq!(
            spec.loaded_tools,
            vec!["exec_bash".to_string(), "read_file".to_string()]
        );

        let toolbox_prompt = render_toolbox_prompt_preview(skill_name, &spec);
        println!(
            "\n[toolbox prompt preview after loading skill `{}`]\n{}\n",
            skill_name, toolbox_prompt
        );
        assert!(toolbox_prompt.contains("\"loaded_skills\""));
        assert!(toolbox_prompt.contains("\"requested_actions\""));
        assert!(toolbox_prompt.contains("\"allow_actions\""));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
