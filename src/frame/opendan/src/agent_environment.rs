use std::collections::HashMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{get_buckyos_api_runtime, MsgRecord, OpenDanAgentSessionRecord};
use log::warn;
use ndn_lib::MsgObject;
use rusqlite::Connection;
use serde_json::{json, Map, Value as Json};
use tokio::fs;
use tokio::sync::Mutex;
use tokio::task;
use upon::Engine;

use buckyos_api::msg_queue::Message;

use crate::agent::{AIAgent, InputQueueKind};
use crate::agent_session::{AgentSession, AgentSessionMgr, SessionInputItem};
use crate::agent_tool::{AgentToolError, AgentToolManager};
use crate::workspace::{
    get_next_ready_todo_code, get_next_ready_todo_text, AgentWorkshop, AgentWorkshopConfig,
};

const MAX_INCLUDE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_RENDER_BYTES: usize = 256 * 1024;
const ESCAPED_OPEN_SENTINEL: &str = "\u{001f}ESCAPED_OPEN_BRACE\u{001f}";
const ESCAPED_CLOSE_SENTINEL: &str = "\u{001f}ESCAPED_CLOSE_BRACE\u{001f}";
const DEFAULT_NEW_MSG_MAX_PULL: usize = 32;
const DEFAULT_SESSION_LIST_MAX_PULL: usize = 16;
const SESSION_RECORD_FILE_NAME: &str = "session.json";

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

    pub fn register_workshop_tools(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Result<(), AgentToolError> {
        self.workshop.register_tools(tool_mgr, session_store)
    }

    // Backward compatibility for old call sites.
    pub fn register_basic_workshop_tools(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Result<(), AgentToolError> {
        self.register_workshop_tools(tool_mgr, session_store)
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
        let (expanded_input, env_ok, env_fail) = expand_opendan_env_tokens(input, env_context);
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
                    .or(clean_optional_text(
                        load_value(placeholder_raw).await?.as_deref(),
                    ))
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

    pub async fn load_value_from_session(
        session: Arc<Mutex<AgentSession>>,
        key: &str,
    ) -> Result<Option<String>, AgentToolError> {
        let k = key.trim();
        let (
            session_id,
            step_index,
            last_step_summary,
            workspace_info,
            session_cwd,
            owner_agent,
            local_workspace_id,
        ) = {
            let guard = session.lock().await;
            (
                guard.session_id.clone(),
                guard.step_index,
                guard.last_step_summary.clone(),
                guard.workspace_info.clone(),
                guard.cwd.clone(),
                guard.owner_agent.clone(),
                guard.local_workspace_id.clone(),
            )
        };

        if k.is_empty() {
            return Ok(None);
        }
        if k == "session_id" {
            return Ok(Some(session_id));
        }
        if k == "step_index" {
            return Ok(Some(step_index.to_string()));
        }
        if k == "last_step_summary" {
            return Ok(last_step_summary);
        }

        if k.starts_with("new_msg") {
            let max_pull = parse_pull_limit_from_key(k, "new_msg", DEFAULT_NEW_MSG_MAX_PULL);
            let kmsg_sub_id = AIAgent::get_session_kmsgqueue_sub_id(
                owner_agent.as_str(),
                session_id.as_str(),
                InputQueueKind::Msg,
            );
            let max_pull_u32 = (max_pull.min(u32::MAX as usize)) as u32;
            let new_msgs =
                AgentSession::pull_new_msg_from_kmsgqueue(kmsg_sub_id.as_str(), max_pull_u32).await;

            if new_msgs.is_err() {
                warn!(
                    "agent_env.load_value_from_session pull_new_msg failed: session={} sub={} err={}",
                    session_id,
                    kmsg_sub_id,
                    new_msgs.err().unwrap()
                );
                return Ok(None);
            }

            let new_msgs = new_msgs.unwrap();
            if new_msgs.is_empty() {
                return Ok(None);
            }

            if let Some(last_msg) = new_msgs.last() {
                let mut guard = session.lock().await;
                // save cursor for ack after process
                guard.just_readed_input_msg = new_msgs.iter().map(|r| r.payload.clone()).collect();
                guard.msg_kmsgqueue_curosr = last_msg.index;
            }

            return Ok(render_new_msgs_from_kmsgqueue(new_msgs.as_slice()).await);
        }

        if k.starts_with("session_list") {
            // k format: session_list / session_list.$num
            let max_pull =
                parse_pull_limit_from_key(k, "session_list", DEFAULT_SESSION_LIST_MAX_PULL);
            return Ok(render_recent_sessions_from_disk(
                &session_cwd,
                session_id.as_str(),
                max_pull,
            )
            .await);
        }

        // if k.starts_with("new_event") {
        //     unimplemented!()
        // }

        if k == "current_todo" || k == "workspace.todolist.next_ready_todo" {
            let value_kind = if k == "current_todo" {
                NextReadyTodoValueKind::TodoCode
            } else {
                NextReadyTodoValueKind::RenderedDetail
            };

            let workspace_id = resolve_session_workspace_id(
                local_workspace_id.as_deref(),
                workspace_info.as_ref(),
            );
            let agent_id = normalize_optional_text(Some(owner_agent.as_str()));
            let todo_db_path = resolve_todo_db_path(
                local_workspace_id.as_deref(),
                workspace_info.as_ref(),
                &session_cwd,
            );

            if let (Some(workspace_id), Some(agent_id), Some(todo_db_path)) =
                (workspace_id, agent_id, todo_db_path)
            {
                if todo_db_path.is_file() {
                    match load_next_ready_todo_value(
                        todo_db_path.clone(),
                        workspace_id.clone(),
                        session_id.clone(),
                        agent_id,
                        value_kind,
                    )
                    .await
                    {
                        Ok(Some(value)) => return Ok(Some(value)),
                        Ok(None) => {}
                        Err(err) => {
                            warn!(
                                "agent_env.load_value_from_session next_ready_todo query failed: session={} workspace={} key={} db={} err={}",
                                session_id,
                                workspace_id,
                                k,
                                todo_db_path.display(),
                                err
                            );
                        }
                    }
                }
            }

            if let Some(workspace_info) = workspace_info.as_ref() {
                return Ok(resolve_workspace_info_text(workspace_info, k));
            }
            return Ok(None);
        }
        if k.starts_with("workspace.") {
            return Ok(workspace_info
                .as_ref()
                .and_then(|ws| resolve_workspace_info_text(ws, k)));
        }

        let workspace_root = extract_workspace_root_from_info(workspace_info.as_ref())
            .or_else(|| non_empty_path(&session_cwd))
            .unwrap_or(std::env::current_dir().map_err(|err| {
                AgentToolError::ExecFailed(format!("read current_dir failed: {err}"))
            })?);
        let cwd_root = non_empty_path(&session_cwd).unwrap_or_else(|| workspace_root.clone());

        //$workspace/readme.txt
        if k.starts_with("$workspace/") {
            let rel_path = &k["$workspace/".len()..];
            return load_text_from_root(workspace_root.as_path(), rel_path).await;
        }
        //$cwd/readme.txt
        if k.starts_with("$cwd/") {
            let rel_path = &k["$cwd/".len()..];
            return load_text_from_root(cwd_root.as_path(), rel_path).await;
        }

        Ok(None)
    }

    pub async fn render_prompt(
        input: &str,
        env_context: &HashMap<String, Json>,
        session: Arc<Mutex<AgentSession>>,
    ) -> Result<AgentTemplateRenderResult, AgentToolError> {
        let session_clone = session.clone();
        Self::render_text_template(
            input,
            |key| {
                let s = session_clone.clone();
                let k = key.to_string();

                async move { Self::load_value_from_session(s, &k).await }
            },
            env_context,
        )
        .await
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

fn parse_pull_limit_from_key(key: &str, prefix: &str, default_pull: usize) -> usize {
    let Some(raw_tail) = key.strip_prefix(prefix) else {
        return default_pull;
    };
    let tail = raw_tail
        .trim()
        .trim_start_matches('.')
        .trim_start_matches('$');
    if tail.is_empty() {
        return default_pull;
    }
    tail.parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.min(4096))
        .unwrap_or(default_pull)
}

async fn render_recent_sessions_from_disk(
    session_cwd: &Path,
    current_session_id: &str,
    max_pull: usize,
) -> Option<String> {
    if max_pull == 0 {
        return None;
    }

    let session_root = resolve_session_root_from_cwd(session_cwd, current_session_id).await?;
    let mut entries = fs::read_dir(&session_root).await.ok()?;
    let mut records = Vec::<OpenDanAgentSessionRecord>::new();

    loop {
        let Ok(Some(entry)) = entries.next_entry().await else {
            break;
        };
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let Some(dir_session_id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let session_file = entry.path().join(SESSION_RECORD_FILE_NAME);
        if !fs::metadata(&session_file)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&session_file).await else {
            continue;
        };
        let Ok(mut record) = serde_json::from_str::<OpenDanAgentSessionRecord>(&raw) else {
            continue;
        };
        if record.session_id.trim().is_empty() {
            record.session_id = dir_session_id;
        }
        if AgentSessionMgr::is_ui_session(record.session_id.as_str()) {
            continue;
        }
        records.push(record);
    }

    if records.is_empty() {
        return None;
    }

    records.sort_by(|a, b| {
        b.last_activity_ms
            .cmp(&a.last_activity_ms)
            .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });

    let items = records
        .into_iter()
        .take(max_pull)
        .map(|record| {
            let title = clean_optional_text(Some(record.title.as_str()))
                .unwrap_or_else(|| format!("Session {}", record.session_id));
            let status = clean_optional_text(Some(record.status.as_str()))
                .unwrap_or_else(|| "normal".to_string());
            let summary = clean_optional_text(Some(record.summary.as_str()))
                .map(|value| collapse_whitespace(value.as_str()))
                .map(|value| truncate_chars(value.as_str(), 200))
                .unwrap_or_default();

            json!({
                "session_id": record.session_id,
                "title": title,
                "summary": summary,
                "status": status,
                "last_activity_ms": record.last_activity_ms,
                "updated_at_ms": record.updated_at_ms,
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&items)
        .ok()
        .and_then(|value| clean_optional_text(Some(value.as_str())))
}

async fn resolve_session_root_from_cwd(
    session_cwd: &Path,
    current_session_id: &str,
) -> Option<PathBuf> {
    let session_id = clean_optional_text(Some(current_session_id))?;
    let mut roots = Vec::<PathBuf>::new();

    if let Some(cwd) = non_empty_path(session_cwd) {
        roots.push(cwd);
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    for root in roots {
        for ancestor in root.ancestors() {
            let session_root = ancestor.join("session");
            let marker = session_root
                .join(session_id.as_str())
                .join(SESSION_RECORD_FILE_NAME);
            if fs::metadata(&marker)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false)
            {
                return Some(session_root);
            }
        }
    }
    None
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if count >= max_chars {
            break;
        }
        output.push(ch);
        count += 1;
    }
    output
}

fn parse_msg_record_from_kmsg_payload(payload: &[u8]) -> Option<MsgRecord> {
    if let Ok(input_item) = serde_json::from_slice::<SessionInputItem>(payload) {
        if let Some(msg_record) = input_item.msg {
            return Some(msg_record);
        }
    }
    serde_json::from_slice::<MsgRecord>(payload).ok()
}

async fn render_new_msgs_from_kmsgqueue(messages: &[Message]) -> Option<String> {
    if messages.is_empty() {
        return None;
    }

    let named_store = match get_buckyos_api_runtime() {
        Ok(runtime) => match runtime.get_named_store().await {
            Ok(store) => Some(store),
            Err(error) => {
                warn!(
                    "agent_env.render_new_msgs init named_store failed: err={}",
                    error
                );
                None
            }
        },
        Err(error) => {
            warn!(
                "agent_env.render_new_msgs runtime is unavailable, fallback without msg object: err={}",
                error
            );
            None
        }
    };

    let mut lines = Vec::<String>::new();
    for message in messages {
        let Some(msg_record) = parse_msg_record_from_kmsg_payload(&message.payload) else {
            warn!(
                "agent_env.render_new_msgs invalid payload: expected SessionInputItem.msg or MsgRecord"
            );
            continue;
        };

        let mut rendered_line = None::<String>;
        if let Some(store) = named_store.as_ref() {
            let msg_id = msg_record.msg_id.clone();
            match store.get_object(&msg_id).await {
                Ok(msg_json) => match serde_json::from_str::<MsgObject>(&msg_json) {
                    Ok(msg_obj) => match serde_json::to_string(&msg_obj) {
                        Ok(line) => rendered_line = Some(line),
                        Err(error) => warn!(
                            "agent_env.render_new_msgs serialize msg object failed: msg_id={} err={}",
                            msg_id, error
                        ),
                    },
                    Err(error) => warn!(
                        "agent_env.render_new_msgs parse msg object failed: msg_id={} err={}",
                        msg_id, error
                    ),
                },
                Err(error) => warn!(
                    "agent_env.render_new_msgs load msg object failed: msg_id={} err={}",
                    msg_id, error
                ),
            }
        }

        if rendered_line.is_none() {
            match serde_json::to_string(&msg_record) {
                Ok(line) => rendered_line = Some(line),
                Err(error) => warn!(
                    "agent_env.render_new_msgs serialize msg record failed: record_id={} err={}",
                    msg_record.record_id, error
                ),
            }
        }

        if let Some(line) = rendered_line {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return None;
    }
    clean_optional_text(Some(lines.join("\n").as_str()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NextReadyTodoValueKind {
    TodoCode,
    RenderedDetail,
}

async fn load_next_ready_todo_value(
    db_path: PathBuf,
    workspace_id: String,
    session_id: String,
    agent_id: String,
    value_kind: NextReadyTodoValueKind,
) -> Result<Option<String>, AgentToolError> {
    task::spawn_blocking(move || {
        let conn = Connection::open(&db_path).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open todo db `{}` failed: {err}",
                db_path.display()
            ))
        })?;
        match value_kind {
            NextReadyTodoValueKind::TodoCode => {
                get_next_ready_todo_code(&conn, &workspace_id, &session_id, &agent_id)
            }
            NextReadyTodoValueKind::RenderedDetail => {
                get_next_ready_todo_text(&conn, &workspace_id, &session_id, &agent_id)
            }
        }
    })
    .await
    .map_err(|err| {
        AgentToolError::ExecFailed(format!("query next ready todo join failed: {err}"))
    })?
}

fn resolve_session_workspace_id(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
) -> Option<String> {
    normalize_optional_text(local_workspace_id)
        .or_else(|| extract_workspace_id_from_json(workspace_info))
}

fn extract_workspace_id_from_json(value: Option<&Json>) -> Option<String> {
    // FIXME(opendan-strong-typing): Weakly-typed compatibility lookup from Json is forbidden.
    // Replace with strongly-typed structs + serde deserialization.
    let value = value?;
    for pointer in [
        "/workspace_id",
        "/local_workspace_id",
        "/id",
        "/workspace/id",
        "/workspace/workspace_id",
        "/workspace/local_workspace_id",
        "/binding/workspace_id",
        "/binding/local_workspace_id",
    ] {
        let parsed = value
            .pointer(pointer)
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty());
        if let Some(workspace_id) = parsed {
            return Some(workspace_id.to_string());
        }
    }
    None
}

fn resolve_todo_db_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    let candidates = collect_workspace_path_candidates(workspace_info, session_cwd);
    let candidate_roots = collect_candidate_ancestors(&candidates);

    if let Some(local_workspace_id) = normalize_optional_text(local_workspace_id) {
        for root in &candidate_roots {
            let local_workspace_path = root
                .join("workspaces")
                .join("local")
                .join(local_workspace_id.as_str());
            let todo_db_path = root.join("todo").join("todo.db");
            if local_workspace_path.is_dir() && todo_db_path.is_file() {
                return Some(todo_db_path);
            }
        }
    }

    for root in &candidate_roots {
        let todo_db_path = root.join("todo").join("todo.db");
        if todo_db_path.is_file() {
            return Some(todo_db_path);
        }
    }

    None
}

fn collect_workspace_path_candidates(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::<PathBuf>::new();
    if let Some(workspace_info) = workspace_info {
        // FIXME(opendan-strong-typing): Weakly-typed compatibility lookup from Json is forbidden.
        // Replace with strongly-typed structs + serde deserialization.
        for pointer in [
            "/workspace_root",
            "/workspace/root",
            "/workspace/root_path",
            "/workspace/path",
            "/workspace/cwd",
            "/workspace/workspace_path",
            "/binding/workspace_path",
            "/binding/workspace_root",
            "/root",
            "/root_path",
            "/path",
            "/workspace_path",
        ] {
            let path = workspace_info
                .pointer(pointer)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(path) = path {
                push_unique_pathbuf(&mut out, PathBuf::from(path));
            }
        }
    }
    if let Some(path) = non_empty_path(session_cwd) {
        push_unique_pathbuf(&mut out, path);
    }
    if out.is_empty() {
        if let Ok(current) = std::env::current_dir() {
            push_unique_pathbuf(&mut out, current);
        }
    }
    out
}

fn collect_candidate_ancestors(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::<PathBuf>::new();
    for path in paths {
        let candidate = if path.is_absolute() {
            path.clone()
        } else if let Ok(current_dir) = std::env::current_dir() {
            current_dir.join(path)
        } else {
            path.clone()
        };

        for ancestor in candidate.ancestors() {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            push_unique_pathbuf(&mut out, ancestor.to_path_buf());
        }
    }
    out
}

fn push_unique_pathbuf(paths: &mut Vec<PathBuf>, value: PathBuf) {
    if value.as_os_str().is_empty() {
        return;
    }
    if paths.iter().any(|item| item == &value) {
        return;
    }
    paths.push(value);
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn resolve_workspace_info_text(workspace_info: &Json, key: &str) -> Option<String> {
    for candidate in workspace_info_path_candidates(key) {
        if let Some(value) = resolve_json_path(workspace_info, candidate.as_str()) {
            if let Some(text) = json_value_to_compact_text(value) {
                return Some(text);
            }
        }
    }
    None
}

fn workspace_info_path_candidates(key: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    push_unique_path(&mut out, key);

    if key == "current_todo" {
        push_unique_path(&mut out, "workspace.current_todo");
    }
    if let Some(stripped) = key.strip_prefix("workspace.") {
        push_unique_path(&mut out, stripped);
    }
    if key == "workspace.todolist" {
        push_unique_path(&mut out, "todolist");
    }
    if let Some(stripped) = key.strip_prefix("workspace.todolist.") {
        let rel_path = format!("todolist.{stripped}");
        push_unique_path(&mut out, rel_path.as_str());
    }
    out
}

fn push_unique_path(paths: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if paths.iter().any(|item| item == trimmed) {
        return;
    }
    paths.push(trimmed.to_string());
}

fn extract_workspace_root_from_info(workspace_info: Option<&Json>) -> Option<PathBuf> {
    // FIXME(opendan-strong-typing): Weakly-typed compatibility lookup from Json is forbidden.
    // Replace with strongly-typed structs + serde deserialization.
    let info = workspace_info?;
    for pointer in [
        "/workspace_root",
        "/workspace/root",
        "/workspace/root_path",
        "/workspace/path",
        "/workspace/cwd",
        "/root",
        "/root_path",
        "/path",
    ] {
        let root = info
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(root) = root {
            return Some(PathBuf::from(root));
        }
    }
    None
}

fn non_empty_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path.to_path_buf())
    }
}

async fn load_text_from_root(
    root: &Path,
    rel_path: &str,
) -> Result<Option<String>, AgentToolError> {
    let rel_path = rel_path.trim();
    if rel_path.is_empty() || !is_safe_relative_path(rel_path) {
        warn!("agent_env.render skip unsafe include: rel_path={rel_path}");
        return Ok(None);
    }

    let absolute_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };
    let include_path = absolute_root.join(rel_path);

    let canonical_root = fs::canonicalize(&absolute_root)
        .await
        .unwrap_or(absolute_root);
    let canonical_path = match fs::canonicalize(&include_path).await {
        Ok(path) => path,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "agent_env.render include resolve failed: path={} err={}",
                    include_path.display(),
                    err
                );
            }
            return Ok(None);
        }
    };
    if !canonical_path.starts_with(&canonical_root) {
        warn!(
            "agent_env.render include escaped root: include={} root={}",
            canonical_path.display(),
            canonical_root.display()
        );
        return Ok(None);
    }

    let bytes = match fs::read(&canonical_path).await {
        Ok(content) => content,
        Err(err) => {
            warn!(
                "agent_env.render include read failed: path={} err={}",
                canonical_path.display(),
                err
            );
            return Ok(None);
        }
    };
    let content = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            warn!(
                "agent_env.render include utf8 decode failed: path={} err={}",
                canonical_path.display(),
                err
            );
            return Ok(None);
        }
    };
    let content = truncate_utf8(&content, MAX_INCLUDE_BYTES);
    Ok(clean_optional_text(Some(content.as_str())))
}

fn resolve_json_path<'a>(value: &'a Json, path: &str) -> Option<&'a Json> {
    let segments: Vec<&str> = path
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    let mut current = value;
    for segment in segments {
        current = match current {
            Json::Object(map) => map.get(segment)?,
            Json::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(current)
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
    async fn render_prompt_uses_session_and_env_context() {
        let session = Arc::new(Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));
        session.lock().await.step_index = 3;
        session.lock().await.workspace_info = Some(json!({ "current_todo": "T01" }));

        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params.x".to_string(), Json::String("env_val".to_string()));

        let result = AgentEnvironment::render_prompt(
            "sid={{session_id}} step={{step_index}} todo={{current_todo}} x=__OPENDAN_ENV(params.x)__",
            &env_context,
            session,
        )
        .await
        .expect("render prompt");

        assert_eq!(result.rendered, "sid=s1 step=3 todo=T01 x=env_val");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.successful_count, 3);
    }

    #[tokio::test]
    async fn render_text_template_expands_env_and_loads_value() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert(
            "params".to_string(),
            json!({ "todo": "T01","priority": "high" }),
        );

        let result = AgentEnvironment::render_text_template(
            "{{workspace.todolist.__OPENDAN_ENV(params.todo)__}}",
            |key| {
                let owned_key = key.to_string();
                async move {
                    if owned_key == "workspace.todolist.T01" {
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
    async fn load_value_from_session_new_msg_respects_numeric_pull_limit() {
        let session = Arc::new(Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));

        let result = AgentEnvironment::load_value_from_session(session, "new_msg.2")
            .await
            .expect("load value from session");
        assert_eq!(result, Some("line-2\nline-3".to_string()));
    }

    #[tokio::test]
    async fn load_value_from_session_new_msg_supports_dollar_pull_limit() {
        let session = Arc::new(Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));

        let result = AgentEnvironment::load_value_from_session(session, "new_msg.$3")
            .await
            .expect("load value from session");
        assert_eq!(result, Some("m2\nm3\nm4".to_string()));
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

        assert_eq!(
            result.rendered,
            "env_ok=E1 env_fail= brace_ok=OK brace_fail="
        );
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

    #[tokio::test]
    async fn load_value_from_session_session_list_defaults_to_16_and_sorts_recent_first() {
        let root = tempdir().expect("create temp dir");
        let session_root = root.path().join("session");
        fs::create_dir_all(&session_root)
            .await
            .expect("create session root");

        async fn write_session_record(
            session_root: &Path,
            session_id: &str,
            title: &str,
            summary: &str,
            last_activity_ms: u64,
        ) {
            let dir = session_root.join(session_id);
            fs::create_dir_all(&dir).await.expect("create session dir");
            let record = OpenDanAgentSessionRecord {
                session_id: session_id.to_string(),
                owner_agent: "did:test:agent".to_string(),
                title: title.to_string(),
                summary: summary.to_string(),
                status: "normal".to_string(),
                created_at_ms: last_activity_ms.saturating_sub(10),
                updated_at_ms: last_activity_ms.saturating_sub(1),
                last_activity_ms,
                links: vec![],
                tags: vec![],
                meta: Json::Object(Map::new()),
            };
            let bytes = serde_json::to_vec_pretty(&record).expect("serialize session record");
            fs::write(dir.join(SESSION_RECORD_FILE_NAME), bytes)
                .await
                .expect("write session file");
        }

        write_session_record(&session_root, "s1", "Session 1", "Summary 1", 100).await;
        write_session_record(&session_root, "s2", "Session 2", "Summary 2", 300).await;
        write_session_record(&session_root, "s3", "Session 3", "Summary 3", 200).await;
        write_session_record(&session_root, "ui-chat-1", "UI Session", "UI Summary", 500).await;

        let session = Arc::new(Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));
        session.lock().await.cwd = root.path().to_path_buf();

        let rendered = AgentEnvironment::load_value_from_session(session.clone(), "session_list")
            .await
            .expect("load session_list")
            .expect("session_list should be rendered");

        let items: Vec<Json> = serde_json::from_str(&rendered).expect("parse rendered list");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["session_id"], "s2");
        assert_eq!(items[1]["session_id"], "s3");
        assert_eq!(items[2]["session_id"], "s1");
        assert!(items.iter().all(|item| item["session_id"] != "ui-chat-1"));

        let rendered_2 = AgentEnvironment::load_value_from_session(session, "session_list.$2")
            .await
            .expect("load session_list.$2")
            .expect("session_list.$2 should be rendered");
        let items_2: Vec<Json> = serde_json::from_str(&rendered_2).expect("parse rendered list");
        assert_eq!(items_2.len(), 2);
        assert_eq!(items_2[0]["session_id"], "s2");
        assert_eq!(items_2[1]["session_id"], "s3");
    }
}
