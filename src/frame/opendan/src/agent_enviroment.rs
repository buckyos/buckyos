use std::collections::HashMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};

use log::warn;
use serde_json::{Map, Value as Json};
use tokio::fs;
use upon::Engine;

use crate::agent_tool::{AgentToolError, ToolManager};
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
    pub session_id: Option<String>,
    pub runtime_kv: Map<String, Json>,
}

#[derive(Clone, Debug)]
pub struct AgentEnvironment {
    workshop: AgentWorkshop,
}

pub struct AgentTemplateRenderResult {
    pub rendered: String,
    /// __OPENDAN_ENV preprocessing: tokens found in env_context
    pub env_expanded: u32,
    /// __OPENDAN_ENV preprocessing: tokens not found in env_context
    pub env_not_found: u32,
    /// {{key}} replacements: successfully resolved
    pub successful_count: u32,
    /// {{key}} replacements: not found
    pub failed_count: u32,
}

impl AgentEnvironment {
    pub async fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, AgentToolError> {
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(workspace_root)).await?;
        Ok(Self { workshop })
    }

    pub fn workspace_root(&self) -> &Path {
        self.workshop.workspace_root()
    }

    pub fn register_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), AgentToolError> {
        self.workshop.register_tools(tool_mgr)
    }

    // Backward compatibility for old call sites.
    pub fn register_basic_workshop_tools(&self, tool_mgr: &ToolManager) -> Result<(), AgentToolError> {
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
            session_id: None,
            runtime_kv: Map::<String, Json>::new(),
        }
    }

    pub async fn render_text_template<F, Fut>(
        input: &str,
        load_value: F,
        env_context: &HashMap<String, Json>,
    ) -> Result<AgentTemplateRenderResult, AgentToolError>
    where
        F: Fn(&str) -> Fut,
        Fut: Future<Output = Result<Option<String>, AgentToolError>>,
    {

        // 1) Expand __OPENDAN_ENV(path)__ from env_context only.
        // 2) Replace {{key}} with env_context value first.
        // 3) If env_context misses, call load_value(key); None => empty string.
        let (expanded_input, env_ok, env_fail) =
            expand_opendan_env_tokens(input, env_context);
        let escaped = escape_template_literals(&expanded_input);

        let mut rebuilt_template = String::new();
        let mut render_ctx = Map::<String, Json>::new();
        let mut slot_seq = 0usize;
        let mut cursor = 0usize;
        let mut brace_ok = 0u32;
        let mut brace_fail = 0u32;

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

            let resolved = if placeholder_raw.is_empty() {
                None
            } else {
                resolve_env_context_value(env_context, placeholder_raw)
                    .and_then(json_value_to_compact_text)
                    .or(clean_optional_text(load_value(placeholder_raw).await?.as_deref()))
            };
            if !placeholder_raw.is_empty() {
                if resolved.is_some() {
                    brace_ok = brace_ok.saturating_add(1);
                } else {
                    brace_fail = brace_fail.saturating_add(1);
                }
            }
            render_ctx.insert(slot_name, Json::String(resolved.unwrap_or_default()));
            cursor = close_pos + 2;
        }

        if cursor < escaped.len() {
            rebuilt_template.push_str(&escaped[cursor..]);
        }

        let mut engine = Engine::new();
        engine
            .add_template("text_template", &rebuilt_template)
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("add text template failed: {err}"))
            })?;

        let mut rendered = engine
            .template("text_template")
            .render(&render_ctx)
            .to_string()
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("render text template failed: {err}"))
            })?;
        rendered = unescape_template_literals(&rendered);
        rendered = truncate_utf8(&rendered, MAX_TOTAL_RENDER_BYTES);

        Ok(AgentTemplateRenderResult {
            rendered,
            env_expanded: env_ok,
            env_not_found: env_fail,
            successful_count: brace_ok,
            failed_count: brace_fail,
        })
    }

    pub async fn render_prompt_template(
        &self,
        template: &str,
        mode: TemplateRenderMode,
        ctx: &PromptTemplateContext,
    ) -> Result<Option<String>, AgentToolError> {
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
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("add prompt template failed: {err}"))
            })?;

        let mut rendered = engine
            .template("prompt_template")
            .render(&render_ctx)
            .to_string()
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("render prompt template failed: {err}"))
            })?;
        rendered = unescape_template_literals(&rendered);
        rendered = truncate_utf8(&rendered, MAX_TOTAL_RENDER_BYTES);
        match mode {
            TemplateRenderMode::Text => Ok(Some(normalize_text_output(&rendered))),
            TemplateRenderMode::InputBlock => Ok(normalize_input_block_output(&rendered)),
        }
    }

    pub async fn render_input_block_template(
        &self,
        template: &str,
        ctx: &PromptTemplateContext,
    ) -> Result<Option<String>, AgentToolError> {
        self.render_prompt_template(template, TemplateRenderMode::InputBlock, ctx)
            .await
    }

    async fn resolve_placeholder(
        &self,
        placeholder_raw: &str,
        ctx: &PromptTemplateContext,
    ) -> Result<Option<String>, AgentToolError> {
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
        "session_id" => clean_optional_text(ctx.session_id.as_deref()),
        _ => ctx
            .runtime_kv
            .get(name)
            .and_then(json_value_to_compact_text),
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

fn expand_opendan_env_tokens(
    input: &str,
    env_context: &HashMap<String, Json>,
) -> (String, u32, u32) {
    const ENV_TOKEN_OPEN: &str = "__OPENDAN_ENV(";
    const ENV_TOKEN_CLOSE: &str = ")__";

    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut found_count = 0u32;
    let mut not_found_count = 0u32;

    while let Some(start) = input[cursor..].find(ENV_TOKEN_OPEN).map(|idx| cursor + idx) {
        output.push_str(&input[cursor..start]);
        let key_start = start + ENV_TOKEN_OPEN.len();
        let Some(key_end) = input[key_start..]
            .find(ENV_TOKEN_CLOSE)
            .map(|idx| key_start + idx)
        else {
            output.push_str(&input[start..]);
            cursor = input.len();
            break;
        };

        let key = input[key_start..key_end].trim();
        let found = !key.is_empty() && resolve_env_context_value(env_context, key).is_some();
        let value = resolve_env_context_value(env_context, key)
            .and_then(json_value_to_compact_text)
            .unwrap_or_default();
        if !key.is_empty() {
            if found {
                found_count = found_count.saturating_add(1);
            } else {
                not_found_count = not_found_count.saturating_add(1);
            }
        }
        output.push_str(&value);
        cursor = key_end + ENV_TOKEN_CLOSE.len();
    }

    if cursor < input.len() {
        output.push_str(&input[cursor..]);
    }
    (output, found_count, not_found_count)
}

fn resolve_env_context_value<'a>(
    env_context: &'a HashMap<String, Json>,
    key: &str,
) -> Option<&'a Json> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(value) = env_context.get(trimmed) {
        return Some(value);
    }

    let path_segments = trimmed
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if path_segments.is_empty() {
        return None;
    }

    for split_idx in (1..=path_segments.len()).rev() {
        let key_prefix = path_segments[..split_idx].join(".");
        let mut current = match env_context.get(key_prefix.as_str()) {
            Some(value) => value,
            None => continue,
        };
        if split_idx == path_segments.len() {
            return Some(current);
        }

        for segment in &path_segments[split_idx..] {
            current = match current {
                Json::Object(map) => map.get(*segment)?,
                Json::Array(items) => {
                    let index = segment.parse::<usize>().ok()?;
                    items.get(index)?
                }
                _ => return None,
            };
        }
        return Some(current);
    }

    None
}

fn is_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-')
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
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn render_text_template_expands_env_and_loads_value() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params".to_string(), json!({ "todo": "T01","priority": "high" }));

        let result = AgentEnvironment::render_text_template(
            "{{workspace/todolist/__OPENDAN_ENV(params.todo)__}}",
            |key| {
                let owned_key = key.to_string();
                async move {
                    if owned_key == "workspace/todolist/T01" {
                        Ok(Some("Do home Work".to_string()))
                    } else {
                        Ok(None)
                    }
                }
            },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "Do home Work");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 0);
    }

    #[tokio::test]
    async fn render_text_template_prefers_env_context_with_json_path() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params".to_string(), json!({ "todo": "T02" }));
        env_context.insert(
            "workspace".to_string(),
            json!({
                "todolist": {
                    "T02": "Do from context"
                }
            }),
        );

        let result = AgentEnvironment::render_text_template(
            "{{workspace.todolist.__OPENDAN_ENV(params.todo)__}}",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "Do from context");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 0);
    }

    #[tokio::test]
    async fn render_text_template_env_not_found_counts_separately() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params.todo".to_string(), Json::String("T01".to_string()));
        // params.missing is NOT in env_context

        let result = AgentEnvironment::render_text_template(
            "a=__OPENDAN_ENV(params.todo)__ b=__OPENDAN_ENV(params.missing)__",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "a=T01 b=");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 1);
        assert_eq!(result.successful_count, 0);
        assert_eq!(result.failed_count, 0);
    }

    #[tokio::test]
    async fn render_text_template_mixed_success_and_fail_for_braces() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("found_key".to_string(), Json::String("value1".to_string()));

        let result = AgentEnvironment::render_text_template(
            "{{found_key}} | {{missing_key}} | {{another_missing}}",
            |key| {
                let k = key.to_string();
                async move {
                    if k == "found_key" {
                        Ok(Some("from_load".to_string()))
                    } else {
                        Ok(None)
                    }
                }
            },
            &env_context,
        )
        .await
        .expect("render text template");

        // found_key: from env_context (preferred over load_value)
        // missing_key, another_missing: load_value returns None -> failed
        assert_eq!(result.rendered, "value1 |  | ");
        assert_eq!(result.env_expanded, 0);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 2);
    }

    #[tokio::test]
    async fn render_text_template_multiple_opendan_env_in_one_placeholder() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("a".to_string(), Json::String("X".to_string()));
        env_context.insert("b".to_string(), Json::String("Y".to_string()));
        env_context.insert("c".to_string(), Json::String("Z".to_string()));

        let result = AgentEnvironment::render_text_template(
            "{{__OPENDAN_ENV(a)__/__OPENDAN_ENV(b)__/__OPENDAN_ENV(c)__}}",
            |key| {
                let k = key.to_string();
                async move {
                    if k == "X/Y/Z" {
                        Ok(Some("nested_value".to_string()))
                    } else {
                        Ok(None)
                    }
                }
            },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "nested_value");
        assert_eq!(result.env_expanded, 3);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 0);
    }

    #[tokio::test]
    async fn render_text_template_all_stats_non_zero() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("ok_env".to_string(), Json::String("E1".to_string()));
        // missing_env is NOT in env_context

        let result = AgentEnvironment::render_text_template(
            "env_ok=__OPENDAN_ENV(ok_env)__ env_fail=__OPENDAN_ENV(missing_env)__ brace_ok={{ok}} brace_fail={{nope}}",
            |key| {
                let k = key.to_string();
                async move {
                    if k == "ok" {
                        Ok(Some("OK".to_string()))
                    } else {
                        Ok(None)
                    }
                }
            },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "env_ok=E1 env_fail= brace_ok=OK brace_fail=");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 1);
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 1);
    }

    #[tokio::test]
    async fn render_text_template_empty_placeholder_not_counted() {
        let env_context = HashMap::<String, Json>::new();

        let result = AgentEnvironment::render_text_template(
            "a={{}}b={{  }}c={{x}}",
            |key| {
                let k = key.to_string();
                async move {
                    if k == "x" {
                        Ok(Some("X".to_string()))
                    } else {
                        Ok(None)
                    }
                }
            },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "a=b=c=X");
        assert_eq!(result.successful_count, 1);
        assert_eq!(result.failed_count, 0);
    }

    #[tokio::test]
    async fn render_text_template_json_path_array_index() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert(
            "data".to_string(),
            json!({
                "items": ["first", "second", "third"],
                "meta": { "count": 3 }
            }),
        );

        let result = AgentEnvironment::render_text_template(
            "{{data.items.0}} | {{data.items.1}} | {{data.meta.count}}",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "first | second | 3");
        assert_eq!(result.successful_count, 3);
        assert_eq!(result.failed_count, 0);
    }

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
    async fn render_text_supports_runtime_kv_and_session_id() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let mut runtime_kv = Map::<String, Json>::new();
        runtime_kv.insert(
            "loop.session_id".to_string(),
            Json::String("S100".to_string()),
        );
        runtime_kv.insert("step.index".to_string(), Json::String("3".to_string()));
        let ctx = PromptTemplateContext {
            session_id: Some("S100".to_string()),
            runtime_kv,
            ..PromptTemplateContext::default()
        };

        let rendered = env
            .render_prompt_template(
                "sid={{session_id}} loop={{loop.session_id}} step={{step.index}}",
                TemplateRenderMode::Text,
                &ctx,
            )
            .await
            .expect("render template")
            .expect("text mode should return string");

        assert_eq!(rendered, "sid=S100 loop=S100 step=3");
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
