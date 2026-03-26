use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::fs;

use crate::agent_tool::{normalize_abs_path, AgentToolError};

pub const SKILLS_REL_PATH: &str = "skills";
const DEFAULT_SKILL_LIST_LIMIT: usize = 16;
const SKILL_META_FILE_NAME: &str = "meta.json";
const SKILL_BODY_FILE_NAME: &str = "skill.md";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSkillRecord {
    pub name: String,
    pub reference: String,
    pub summary: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSkillSpec {
    pub name: String,
    pub reference: String,
    pub summary: String,
    pub content: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
struct AgentSkillMeta {
    pub name: String,
    pub summary: String,
    #[serde(alias = "introduce")]
    pub introduce: String,
    #[serde(alias = "description")]
    pub description: String,
}

impl AgentSkillMeta {
    fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.summary = self.summary.trim().to_string();
        self.introduce = self.introduce.trim().to_string();
        self.description = self.description.trim().to_string();
    }

    fn summary_text(&self) -> String {
        if !self.summary.is_empty() {
            return self.summary.clone();
        }
        if !self.introduce.is_empty() {
            return self.introduce.clone();
        }
        self.description.clone()
    }
}

pub async fn merge_skill_records_from_dir(
    skills_root: &Path,
    records: &mut HashMap<String, AgentSkillRecord>,
) -> Result<(), AgentToolError> {
    for record in discover_skill_records_in_root(skills_root).await? {
        records.insert(record.reference.clone(), record);
    }
    Ok(())
}

pub async fn list_skill_records_from_roots(
    skill_roots: &[PathBuf],
) -> Result<Vec<AgentSkillRecord>, AgentToolError> {
    let mut records = HashMap::<String, AgentSkillRecord>::new();
    for root in skill_roots {
        merge_skill_records_from_dir(root, &mut records).await?;
    }
    let mut out = records.into_values().collect::<Vec<_>>();
    out.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(out)
}

pub async fn load_skill_from_root(
    skills_root: &Path,
    skill_ref: &str,
) -> Result<AgentSkillSpec, AgentToolError> {
    load_skill_from_roots(&[skills_root.to_path_buf()], skills_root, skill_ref).await
}

pub async fn load_skill_from_roots(
    skill_roots: &[PathBuf],
    cwd: &Path,
    skill_ref: &str,
) -> Result<AgentSkillSpec, AgentToolError> {
    let skill_ref = skill_ref.trim();
    if skill_ref.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "skill reference cannot be empty".to_string(),
        ));
    }

    if is_explicit_skill_path(skill_ref) {
        return load_skill_from_path(cwd, skill_ref, Some(skill_ref.to_string())).await;
    }

    for root in skill_roots.iter().rev() {
        if let Some(skill_dir) = find_skill_dir_by_reference(root, skill_ref).await? {
            return load_skill_from_dir(skill_dir, Some(skill_ref.to_string())).await;
        }
    }

    if looks_like_path(skill_ref) {
        return load_skill_from_path(cwd, skill_ref, Some(skill_ref.to_string())).await;
    }

    Err(AgentToolError::NotFound(format!(
        "skill not found: {skill_ref}"
    )))
}

pub fn normalize_unique_skill_refs(items: Vec<String>) -> Vec<String> {
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

pub fn move_skill_ref_to_front(items: &mut Vec<String>, skill_ref: &str) {
    let skill_ref = skill_ref.trim();
    if skill_ref.is_empty() {
        return;
    }
    items.retain(|item| item.trim() != skill_ref);
    items.insert(0, skill_ref.to_string());
}

pub fn remove_skill_ref(items: &mut Vec<String>, skill_ref: &str) {
    let skill_ref = skill_ref.trim();
    if skill_ref.is_empty() {
        return;
    }
    items.retain(|item| item.trim() != skill_ref);
}

pub fn render_skill_records_for_prompt(
    mut records: Vec<AgentSkillRecord>,
    recent_refs: &[String],
    loaded_refs: &[String],
    limit: usize,
) -> String {
    if records.is_empty() {
        return String::new();
    }

    let limit = if limit == 0 {
        DEFAULT_SKILL_LIST_LIMIT
    } else {
        limit
    };
    let mut weight = HashMap::<String, usize>::new();
    for (idx, item) in recent_refs.iter().enumerate() {
        weight.insert(item.trim().to_string(), idx);
    }
    let loaded = loaded_refs
        .iter()
        .map(|item| item.trim().to_string())
        .collect::<HashSet<_>>();

    records.sort_by(|left, right| {
        let left_rank = weight
            .get(left.reference.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let right_rank = weight
            .get(right.reference.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.name.cmp(&right.name))
    });

    records
        .into_iter()
        .take(limit)
        .map(|item| {
            let loaded_tag = if loaded.contains(item.reference.as_str()) {
                " [loaded]"
            } else {
                ""
            };
            if item.summary.is_empty() {
                format!("- {}{}", item.name, loaded_tag)
            } else {
                format!("- {}{}: {}", item.name, loaded_tag, item.summary)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn render_loaded_skills_content(
    skill_roots: &[PathBuf],
    cwd: &Path,
    loaded_refs: &[String],
) -> Result<String, AgentToolError> {
    let mut rendered = Vec::<String>::new();
    for skill_ref in normalize_unique_skill_refs(loaded_refs.to_vec()) {
        let spec = load_skill_from_roots(skill_roots, cwd, skill_ref.as_str()).await?;
        let content = spec.content.trim();
        if content.is_empty() {
            continue;
        }
        rendered.push(content.to_string());
    }
    Ok(rendered.join("\n\n"))
}

async fn discover_skill_records_in_root(
    skills_root: &Path,
) -> Result<Vec<AgentSkillRecord>, AgentToolError> {
    let mut out = Vec::<AgentSkillRecord>::new();
    if !fs::try_exists(skills_root)
        .await
        .map_err(|err| io_error("check skills dir", skills_root, err))?
    {
        return Ok(out);
    }

    for skill_dir in discover_skill_dirs(skills_root).await? {
        let spec = build_skill_spec_from_root(skills_root, skill_dir).await?;
        out.push(AgentSkillRecord {
            name: spec.name,
            reference: spec.reference,
            summary: spec.summary,
            path: spec.path,
        });
    }

    Ok(out)
}

async fn discover_skill_dirs(skills_root: &Path) -> Result<Vec<PathBuf>, AgentToolError> {
    let mut dirs = Vec::<PathBuf>::new();
    let mut pending = vec![skills_root.to_path_buf()];

    while let Some(dir) = pending.pop() {
        if !fs::try_exists(&dir)
            .await
            .map_err(|err| io_error("check skill dir", &dir, err))?
        {
            continue;
        }

        let mut entries = fs::read_dir(&dir)
            .await
            .map_err(|err| io_error("read skill dir", &dir, err))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|err| io_error("read skill dir entry", &dir, err))?
        {
            let path = entry.path();
            let file_name = entry
                .file_name()
                .to_str()
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            if file_name.is_empty() || file_name.starts_with('.') {
                continue;
            }

            let metadata = entry
                .metadata()
                .await
                .map_err(|err| io_error("read skill metadata", &path, err))?;
            if metadata.is_dir() {
                if is_skill_dir(&path).await? {
                    dirs.push(path);
                    continue;
                }
                pending.push(path);
            }
        }
    }

    dirs.sort();
    Ok(dirs)
}

async fn find_skill_dir_by_reference(
    skills_root: &Path,
    skill_ref: &str,
) -> Result<Option<PathBuf>, AgentToolError> {
    if !fs::try_exists(skills_root)
        .await
        .map_err(|err| io_error("check skills dir", skills_root, err))?
    {
        return Ok(None);
    }

    for path in discover_skill_dirs(skills_root).await? {
        let Some(relative) = path.strip_prefix(skills_root).ok() else {
            continue;
        };
        if path_to_reference(relative) == skill_ref.trim() {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

async fn build_skill_spec_from_root(
    skills_root: &Path,
    skill_dir: PathBuf,
) -> Result<AgentSkillSpec, AgentToolError> {
    let relative = skill_dir.strip_prefix(skills_root).map_err(|_| {
        AgentToolError::ExecFailed(format!(
            "skill path `{}` is outside root `{}`",
            skill_dir.display(),
            skills_root.display()
        ))
    })?;
    let reference = path_to_reference(relative);
    load_skill_from_dir(skill_dir, Some(reference)).await
}

fn summarize_skill_content(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

fn looks_like_path(skill_ref: &str) -> bool {
    let trimmed = skill_ref.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.contains('\\')
        || trimmed.contains('/')
}

fn is_explicit_skill_path(skill_ref: &str) -> bool {
    let trimmed = skill_ref.trim();
    trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.contains('\\')
}

fn normalize_skill_path(cwd: &Path, skill_ref: &str) -> PathBuf {
    let path = Path::new(skill_ref.trim());
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_abs_path(&joined)
}

async fn load_skill_from_path(
    cwd: &Path,
    skill_ref: &str,
    reference_override: Option<String>,
) -> Result<AgentSkillSpec, AgentToolError> {
    let normalized = normalize_skill_path(cwd, skill_ref);
    let metadata = fs::metadata(&normalized)
        .await
        .map_err(|err| io_error("read skill path metadata", &normalized, err))?;
    if metadata.is_dir() {
        return load_skill_from_dir(normalized, reference_override).await;
    }
    let Some(file_name) = normalized.file_name().and_then(|value| value.to_str()) else {
        return Err(AgentToolError::NotFound(format!(
            "skill file not found: {}",
            normalized.display()
        )));
    };
    if file_name != SKILL_BODY_FILE_NAME {
        return Err(AgentToolError::InvalidArgs(format!(
            "skill file path must point to `{SKILL_BODY_FILE_NAME}`: {}",
            normalized.display()
        )));
    }
    let parent = normalized.parent().ok_or_else(|| {
        AgentToolError::NotFound(format!(
            "skill directory not found: {}",
            normalized.display()
        ))
    })?;
    load_skill_from_dir(parent.to_path_buf(), reference_override).await
}

async fn load_skill_from_dir(
    skill_dir: PathBuf,
    reference_override: Option<String>,
) -> Result<AgentSkillSpec, AgentToolError> {
    let skill_dir = normalize_abs_path(&skill_dir);
    if !is_skill_dir(&skill_dir).await? {
        return Err(AgentToolError::NotFound(format!(
            "skill directory not found or invalid: {}",
            skill_dir.display()
        )));
    }

    let meta = read_skill_meta(&skill_dir).await?;
    let raw = read_skill_body(&skill_dir).await?;
    let content = raw.trim().to_string();
    let mut name = meta.name.trim().to_string();
    if name.is_empty() {
        name = file_stem_or_fallback(skill_dir.as_path(), "skill");
    }
    let mut summary = meta.summary_text();
    if summary.is_empty() {
        summary = summarize_skill_content(content.as_str());
    }
    let reference = reference_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| name.clone());

    Ok(AgentSkillSpec {
        name,
        reference,
        summary,
        content,
        path: skill_dir.display().to_string(),
    })
}

async fn is_skill_dir(path: &Path) -> Result<bool, AgentToolError> {
    let meta_path = path.join(SKILL_META_FILE_NAME);
    let body_path = path.join(SKILL_BODY_FILE_NAME);
    let has_meta = fs::try_exists(&meta_path)
        .await
        .map_err(|err| io_error("check skill meta", &meta_path, err))?;
    let has_body = fs::try_exists(&body_path)
        .await
        .map_err(|err| io_error("check skill body", &body_path, err))?;
    Ok(has_meta && has_body)
}

async fn read_skill_meta(skill_dir: &Path) -> Result<AgentSkillMeta, AgentToolError> {
    let meta_path = skill_dir.join(SKILL_META_FILE_NAME);
    let raw = fs::read_to_string(&meta_path)
        .await
        .map_err(|err| io_error("read skill meta", &meta_path, err))?;
    let mut meta = serde_json::from_str::<AgentSkillMeta>(&raw).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "parse skill meta `{}` failed: {err}",
            meta_path.display()
        ))
    })?;
    meta.normalize();
    Ok(meta)
}

async fn read_skill_body(skill_dir: &Path) -> Result<String, AgentToolError> {
    let body_path = skill_dir.join(SKILL_BODY_FILE_NAME);
    fs::read_to_string(&body_path)
        .await
        .map_err(|err| io_error("read skill body", &body_path, err))
}

fn path_to_reference(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn file_stem_or_fallback(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn io_error(action: &str, path: &Path, source: std::io::Error) -> AgentToolError {
    AgentToolError::ExecFailed(format!("{action} `{}` failed: {source}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn load_skill_from_root_reads_meta_json_and_skill_md() {
        let temp = tempdir().expect("create temp dir");
        let skill_dir = temp.path().join("planner");
        fs::create_dir_all(&skill_dir)
            .await
            .expect("create skill dir");
        fs::write(
            skill_dir.join("meta.json"),
            r#"{"name":"planner","summary":"planning helper"}"#,
        )
        .await
        .expect("write skill meta");
        fs::write(skill_dir.join("skill.md"), "do plan first")
            .await
            .expect("write skill body");

        let spec = load_skill_from_root(temp.path(), "planner")
            .await
            .expect("load skill");
        assert_eq!(spec.name, "planner");
        assert_eq!(spec.reference, "planner");
        assert_eq!(spec.summary, "planning helper");
        assert_eq!(spec.content, "do plan first");
    }

    #[tokio::test]
    async fn list_skill_records_merges_roots_and_workspace_overrides_agent() {
        let temp = tempdir().expect("create temp dir");
        let agent_root = temp.path().join("agent-skills");
        let workspace_root = temp.path().join("workspace-skills");
        fs::create_dir_all(agent_root.join("shared"))
            .await
            .expect("create agent skill");
        fs::create_dir_all(workspace_root.join("shared"))
            .await
            .expect("create workspace skill");
        fs::create_dir_all(workspace_root.join("ws_only"))
            .await
            .expect("create workspace only skill");

        fs::write(
            agent_root.join("shared/meta.json"),
            r#"{"name":"shared","summary":"agent shared"}"#,
        )
        .await
        .expect("write agent skill meta");
        fs::write(
            agent_root.join("shared/skill.md"),
            "# shared\nagent version",
        )
        .await
        .expect("write agent skill body");
        fs::write(
            workspace_root.join("shared/meta.json"),
            r#"{"name":"shared","summary":"workspace shared"}"#,
        )
        .await
        .expect("write workspace shared meta");
        fs::write(
            workspace_root.join("shared/skill.md"),
            "# shared\nworkspace version",
        )
        .await
        .expect("write workspace shared body");
        fs::write(
            workspace_root.join("ws_only/meta.json"),
            r#"{"name":"ws_only","summary":"workspace only skill"}"#,
        )
        .await
        .expect("write workspace only meta");
        fs::write(
            workspace_root.join("ws_only/skill.md"),
            "workspace only skill",
        )
        .await
        .expect("write workspace only body");

        let records = list_skill_records_from_roots(&[agent_root, workspace_root])
            .await
            .expect("list skills");
        assert_eq!(records.len(), 2);

        let mut by_name = HashMap::<String, String>::new();
        for record in records {
            by_name.insert(record.name, record.summary);
        }
        assert_eq!(by_name.get("shared"), Some(&"workspace shared".to_string()));
        assert_eq!(
            by_name.get("ws_only"),
            Some(&"workspace only skill".to_string())
        );
    }

    #[test]
    fn render_skill_records_marks_loaded_and_uses_recent_order() {
        let rendered = render_skill_records_for_prompt(
            vec![
                AgentSkillRecord {
                    name: "alpha".to_string(),
                    reference: "alpha".to_string(),
                    summary: "first".to_string(),
                    path: "/tmp/alpha".to_string(),
                },
                AgentSkillRecord {
                    name: "beta".to_string(),
                    reference: "beta".to_string(),
                    summary: "second".to_string(),
                    path: "/tmp/beta".to_string(),
                },
            ],
            &["beta".to_string()],
            &["beta".to_string()],
            8,
        );

        assert_eq!(rendered.lines().next(), Some("- beta [loaded]: second"));
    }
}
