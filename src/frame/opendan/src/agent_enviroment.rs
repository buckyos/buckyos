use std::path::{Component, Path, PathBuf};

use log::warn;
use serde_json::{Map, Value as Json};
use tokio::fs;
use upon::Engine;

use crate::agent_tool::{ToolError, ToolManager};
use crate::workspace::{AgentWorkshop, AgentWorkshopConfig};

const MAX_INCLUDE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_RENDER_BYTES: usize = 256 * 1024;
const ESCAPED_OPEN_SENTINEL: &str = "\u{001f}ESCAPED_OPEN_BRACE\u{001f}";
const ESCAPED_CLOSE_SENTINEL: &str = "\u{001f}ESCAPED_CLOSE_BRACE\u{001f}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateRenderMode {
    Text,
    InputBlock,
}

#[derive(Clone, Debug, Default)]
pub struct PromptTemplateContext {
    pub new_msg: Option<String>,
    pub new_event: Option<String>,
    pub cwd_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct AgentEnvironment {
    workshop: AgentWorkshop,
}

impl AgentEnvironment {
    pub async fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(workspace_root)).await?;
        Ok(Self { workshop })
    }

    pub fn workspace_root(&self) -> &Path {
        self.workshop.workspace_root()
    }

    pub fn register_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        self.workshop.register_tools(tool_mgr)
    }

    // Backward compatibility for old call sites.
    pub fn register_basic_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        self.register_workshop_tools(tool_mgr)
    }

    pub fn build_prompt_template_context(
        &self,
        payload: &Json,
        cwd_path: Option<PathBuf>,
    ) -> PromptTemplateContext {
        PromptTemplateContext {
            new_msg: extract_input_source(payload, "new_msg", "inbox"),
            new_event: extract_input_source(payload, "new_event", "events"),
            cwd_path,
        }
    }

    pub async fn render_prompt_template(
        &self,
        template: &str,
        mode: TemplateRenderMode,
        ctx: &PromptTemplateContext,
    ) -> Result<Option<String>, ToolError> {
        if template.trim().is_empty() {
            return Ok(match mode {
                TemplateRenderMode::Text => Some(String::new()),
                TemplateRenderMode::InputBlock => None,
            });
        }

        let escaped = escape_template_literals(template);
        let mut rebuilt_template = String::new();
        let mut render_ctx = Map::<String, Json>::new();
        let mut slot_seq = 0usize;
        let mut cursor = 0usize;

        while let Some(open_pos) = escaped[cursor..].find("{{").map(|idx| cursor + idx) {
            rebuilt_template.push_str(&escaped[cursor..open_pos]);
            let content_start = open_pos + 2;
            let Some(close_pos) = escaped[content_start..]
                .find("}}")
                .map(|idx| content_start + idx)
            else {
                rebuilt_template.push_str(&escaped[open_pos..]);
                cursor = escaped.len();
                break;
            };

            let placeholder_raw = escaped[content_start..close_pos].trim();
            let slot_name = format!("slot_{slot_seq}");
            slot_seq = slot_seq.saturating_add(1);
            rebuilt_template.push_str("{{");
            rebuilt_template.push_str(&slot_name);
            rebuilt_template.push_str("}}");

            let resolved = self.resolve_placeholder(placeholder_raw, ctx).await?;
            render_ctx.insert(slot_name, Json::String(resolved.unwrap_or_default()));

            cursor = close_pos + 2;
        }

        if cursor < escaped.len() {
            rebuilt_template.push_str(&escaped[cursor..]);
        }

        let mut engine = Engine::new();
        engine
            .add_template("prompt_template", &rebuilt_template)
            .map_err(|err| ToolError::ExecFailed(format!("add prompt template failed: {err}")))?;

        let mut rendered = engine
            .template("prompt_template")
            .render(&render_ctx)
            .to_string()
            .map_err(|err| {
                ToolError::ExecFailed(format!("render prompt template failed: {err}"))
            })?;
        rendered = unescape_template_literals(&rendered);
        rendered = truncate_utf8(&rendered, MAX_TOTAL_RENDER_BYTES);

        match mode {
            TemplateRenderMode::Text => Ok(Some(normalize_text_output(&rendered))),
            TemplateRenderMode::InputBlock => Ok(normalize_input_block_output(&rendered)),
        }
    }

    async fn resolve_placeholder(
        &self,
        placeholder_raw: &str,
        ctx: &PromptTemplateContext,
    ) -> Result<Option<String>, ToolError> {
        let placeholder = placeholder_raw.trim();
        if placeholder.is_empty() {
            return Ok(None);
        }

        if is_variable_name(placeholder) {
            return Ok(resolve_variable(placeholder, ctx));
        }

        let Some((ns, rel_path_raw)) = placeholder.split_once('/') else {
            return Ok(None);
        };

        let ns = ns.trim();
        let rel_path = rel_path_raw.trim();
        if rel_path.is_empty() {
            return Ok(None);
        }

        let root = match ns {
            "workspace" => self.workspace_root().to_path_buf(),
            "cwd" => ctx
                .cwd_path
                .clone()
                .unwrap_or_else(|| self.workspace_root().to_path_buf()),
            _ => return Ok(None),
        };

        if !is_safe_relative_path(rel_path) {
            warn!(
                "agent_env.template skip unsafe include: ns={} rel_path={}",
                ns, rel_path
            );
            return Ok(None);
        }

        let include_path = root.join(rel_path);
        let canonical_root = fs::canonicalize(&root).await.unwrap_or(root);
        let canonical_path = match fs::canonicalize(&include_path).await {
            Ok(path) => path,
            Err(err) => {
                warn!(
                    "agent_env.template include not found: path={} err={}",
                    include_path.display(),
                    err
                );
                return Ok(None);
            }
        };
        if !canonical_path.starts_with(&canonical_root) {
            warn!(
                "agent_env.template include escaped root: include={} root={}",
                canonical_path.display(),
                canonical_root.display()
            );
            return Ok(None);
        }

        let bytes = match fs::read(&canonical_path).await {
            Ok(content) => content,
            Err(err) => {
                warn!(
                    "agent_env.template include read failed: path={} err={}",
                    canonical_path.display(),
                    err
                );
                return Ok(None);
            }
        };
        let content = match String::from_utf8(bytes) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "agent_env.template include utf8 decode failed: path={} err={}",
                    canonical_path.display(),
                    err
                );
                return Ok(None);
            }
        };

        let content = truncate_utf8(&content, MAX_INCLUDE_BYTES);
        Ok(clean_optional_text(Some(content.as_str())))
    }
}

fn resolve_variable(name: &str, ctx: &PromptTemplateContext) -> Option<String> {
    match name {
        "new_msg" => clean_optional_text(ctx.new_msg.as_deref()),
        "new_event" => clean_optional_text(ctx.new_event.as_deref()),
        _ => None,
    }
}

fn extract_input_source(payload: &Json, scalar_key: &str, array_key: &str) -> Option<String> {
    if let Some(v) = payload.get(scalar_key) {
        return json_value_to_compact_text(v);
    }

    let lines = payload
        .get(array_key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(json_value_to_compact_text)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn json_value_to_compact_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::String(v) => clean_optional_text(Some(v)),
        _ => serde_json::to_string(value)
            .ok()
            .and_then(|text| clean_optional_text(Some(text.as_str()))),
    }
}

fn is_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_safe_relative_path(path: &str) -> bool {
    let rel = Path::new(path);
    if rel.as_os_str().is_empty() || rel.is_absolute() {
        return false;
    }
    rel.components().all(|component| {
        !matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn clean_optional_text(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn escape_template_literals(template: &str) -> String {
    template
        .replace(r"\{{", ESCAPED_OPEN_SENTINEL)
        .replace(r"\}}", ESCAPED_CLOSE_SENTINEL)
}

fn unescape_template_literals(rendered: &str) -> String {
    rendered
        .replace(ESCAPED_OPEN_SENTINEL, "{{")
        .replace(ESCAPED_CLOSE_SENTINEL, "}}")
}

fn normalize_text_output(text: &str) -> String {
    let mut output = Vec::<String>::new();
    let mut empty_run = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            empty_run = empty_run.saturating_add(1);
            if empty_run > 2 {
                continue;
            }
            output.push(String::new());
            continue;
        }
        empty_run = 0;
        output.push(trimmed.to_string());
    }
    output.join("\n").trim().to_string()
}

fn normalize_input_block_output(text: &str) -> Option<String> {
    let mut lines = Vec::<String>::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(trimmed.to_string());
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn render_text_replaces_variables() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext {
            new_msg: Some("hello".to_string()),
            ..PromptTemplateContext::default()
        };

        let rendered = env
            .render_prompt_template("A={{new_msg}}", TemplateRenderMode::Text, &ctx)
            .await
            .expect("render template")
            .expect("text mode should return string");

        assert_eq!(rendered, "A=hello");
    }

    #[tokio::test]
    async fn render_input_block_returns_none_when_empty() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext::default();

        let rendered = env
            .render_prompt_template("{{new_msg}}", TemplateRenderMode::InputBlock, &ctx)
            .await
            .expect("render template");

        assert_eq!(rendered, None);
    }

    #[tokio::test]
    async fn render_input_block_merges_multiline_sources() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext {
            new_event: Some("evt".to_string()),
            new_msg: Some("msg".to_string()),
            ..PromptTemplateContext::default()
        };

        let rendered = env
            .render_prompt_template(
                "{{new_event}}\n{{new_msg}}",
                TemplateRenderMode::InputBlock,
                &ctx,
            )
            .await
            .expect("render template")
            .expect("should produce input");

        assert_eq!(rendered, "evt\nmsg");
    }

    #[tokio::test]
    async fn render_workspace_include_reads_file_content() {
        let root = tempdir().expect("create temp dir");
        let include_path = root.path().join("to_agent.md");
        fs::write(&include_path, "hello include")
            .await
            .expect("write include file");

        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext::default();
        let rendered = env
            .render_prompt_template("{{workspace/to_agent.md}}", TemplateRenderMode::Text, &ctx)
            .await
            .expect("render template")
            .expect("text mode should return string");

        assert_eq!(rendered, "hello include");
    }

    #[tokio::test]
    async fn render_rejects_path_traversal_include() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext::default();
        let rendered = env
            .render_prompt_template(
                "{{workspace/../secret.txt}}",
                TemplateRenderMode::Text,
                &ctx,
            )
            .await
            .expect("render template")
            .expect("text mode should return string");

        assert_eq!(rendered, "");
    }
}
