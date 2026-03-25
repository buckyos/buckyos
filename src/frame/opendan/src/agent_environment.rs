use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{
    get_buckyos_api_runtime, Contact, MsgRecord, OpenDanAgentSessionRecord, TaskManagerClient,
};
use chrono::{DateTime, Datelike, Timelike, Utc};
use log::{debug, warn};
use name_lib::DID;
use ndn_lib::MsgObject;
use rusqlite::{params, Connection};
use serde_json::{Map, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task;
use tokio::time::{timeout, Duration};
use upon::Engine;

use buckyos_api::msg_queue::Message;

use crate::agent::{AIAgent, InputQueueKind};
use crate::agent_session::{AgentSession, AgentSessionMgr, SessionInputItem};
use crate::agent_tool::{
    get_next_ready_todo_code, get_next_ready_todo_text, get_session_todo_text_by_ref,
    AgentToolError, AgentToolManager,
};
use crate::step_record::LLMStepPromptRenderOptions;
use crate::workspace::{
    AgentWorkshop, AgentWorkshopConfig, LocalWorkspaceManager, WorkshopIndex, WorkspaceType,
};
use crate::workspace_path::{
    non_empty_path, resolve_agent_env_root, resolve_agent_env_root_from_local_workspace_hint,
    resolve_bound_workspace_id, resolve_default_local_workspace_path,
    resolve_session_workspace_root, resolve_workspace_binding_ref, WORKSHOP_INDEX_FILE_NAME,
    WORKSHOP_TODO_DB_REL_PATH,
};

const MAX_INCLUDE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_RENDER_BYTES: usize = 256 * 1024;
const TEMPLATE_EXEC_TIMEOUT_MS: u64 = 10_000;
const ESCAPED_OPEN_SENTINEL: &str = "\u{001f}ESCAPED_OPEN_BRACE\u{001f}";
const ESCAPED_CLOSE_SENTINEL: &str = "\u{001f}ESCAPED_CLOSE_BRACE\u{001f}";
const DEFAULT_NEW_MSG_MAX_PULL: usize = 32;
const DEFAULT_NEW_EVENT_MAX_PULL: usize = 32;
const DEFAULT_SESSION_LIST_MAX_PULL: usize = 16;
const DEFAULT_LOCAL_WORKSPACE_LIST_MAX_PULL: usize = 16;
const DEFAULT_LAST_STEPS_MAX_PULL: usize = 8;
const DEFAULT_WORKSPACE_TODO_LIST_MAX_ITEMS: usize = 64;
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

#[derive(Debug)]
pub struct AgentTemplateRenderResult {
    pub rendered: String,
    pub env_expanded: u32,
    pub env_not_found: u32,
    pub content_loaded: u32,
    pub content_failed: u32,
    pub var_registered: u32,
    pub var_failed: u32,
    pub resolved_vars: HashMap<String, bool>,
}

impl AgentEnvironment {
    pub async fn new(agent_env_root: impl Into<PathBuf>) -> Result<Self, AgentToolError> {
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(agent_env_root)).await?;
        Ok(Self { workshop })
    }

    pub fn agent_env_root(&self) -> &Path {
        self.workshop.agent_env_root()
    }

    pub fn local_workspace_manager(&self) -> &LocalWorkspaceManager {
        self.workshop.local_workspace_manager()
    }

    pub fn register_workshop_tools(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Result<(), AgentToolError> {
        self.workshop.register_tools(tool_mgr, session_store)
    }

    pub fn register_workshop_tools_with_task_mgr(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
        task_mgr: Arc<TaskManagerClient>,
    ) -> Result<(), AgentToolError> {
        self.workshop
            .register_tools_with_task_mgr(tool_mgr, session_store, Some(task_mgr))
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
        const MAX_PREPROCESS_PASSES: usize = 32;

        let mut preprocessed = input.to_string();
        let mut render_ctx = Map::<String, Json>::new();
        let mut env_expanded = 0u32;
        let mut env_not_found = 0u32;
        let mut content_loaded = 0u32;
        let mut content_failed = 0u32;
        let mut var_registered = 0u32;
        let mut var_failed = 0u32;
        let mut resolved_vars = HashMap::<String, bool>::new();

        for _ in 0..MAX_PREPROCESS_PASSES {
            let (
                next_template,
                changed,
                pass_env_expanded,
                pass_env_not_found,
                pass_content_loaded,
                pass_content_failed,
                pass_var_registered,
                pass_var_failed,
            ) = preprocess_text_template_pass(
                preprocessed.as_str(),
                &load_value,
                env_context,
                &mut render_ctx,
                &mut resolved_vars,
            )
            .await?;

            preprocessed = next_template;
            env_expanded = env_expanded.saturating_add(pass_env_expanded);
            env_not_found = env_not_found.saturating_add(pass_env_not_found);
            content_loaded = content_loaded.saturating_add(pass_content_loaded);
            content_failed = content_failed.saturating_add(pass_content_failed);
            var_registered = var_registered.saturating_add(pass_var_registered);
            var_failed = var_failed.saturating_add(pass_var_failed);

            if !changed {
                break;
            }
        }

        if preprocessed.contains("__OPENDAN_") {
            return Err(AgentToolError::ExecFailed(
                "render text template failed: unresolved __OPENDAN_* directive remains after preprocessing"
                    .to_string(),
            ));
        }

        let escaped = escape_template_literals(&preprocessed);
        let mut engine = Engine::new();
        engine
            .add_template("text_template", &escaped)
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
            env_expanded,
            env_not_found,
            content_loaded,
            content_failed,
            var_registered,
            var_failed,
            resolved_vars,
        })
    }

    pub async fn load_value_from_session(
        session: Arc<Mutex<AgentSession>>,
        key: &str,
    ) -> Result<Option<String>, AgentToolError> {
        let k = key.trim();
        let (
            session_id,
            session_title,
            behavior_name,
            step_index,
            step_num,
            workspace_info,
            session_cwd,
            session_root_dir,
            owner_agent,
            local_workspace_id,
        ) = {
            let guard = session.lock().await;
            (
                guard.session_id.clone(),
                guard.title.clone(),
                guard.current_behavior.clone(),
                guard.step_index,
                guard.step_num,
                guard.workspace_info.clone(),
                guard.pwd.clone(),
                guard.session_root_dir.clone(),
                guard.owner_agent.clone(),
                guard.local_workspace_id.clone(),
            )
        };

        if k.is_empty() {
            return Ok(None);
        }
        if k == "step_record" || k == "llm_step_record" {
            let mut guard = session.lock().await;
            return guard
                .render_llm_step_records_prompt(None)
                .await
                .map(|text| clean_optional_text(Some(text.as_str())));
        }
        if k == "step_record.last" || k == "llm_step_record.last" {
            let mut guard = session.lock().await;
            return guard.render_last_llm_step_record().await;
        }
        if k == "step_record.path" || k == "llm_step_record.path" || k == "step_record_path" {
            let mut guard = session.lock().await;
            return Ok(guard
                .llm_step_record_path()
                .map(|path| path.display().to_string()));
        }
        if k == "session_id" {
            return Ok(Some(session_id));
        }
        if k == "session_title" {
            return Ok(clean_optional_text(Some(session_title.as_str())));
        }
        if k == "step_index" {
            return Ok(Some(step_index.to_string()));
        }
        if k == "step_num" {
            return Ok(Some(step_num.to_string()));
        }
        if k == "current_behavior" || k == "behavior_name" {
            return Ok(Some(behavior_name));
        }
        if k == "owner" || k.starts_with("owner.") {
            return Ok(load_owner_value_for_prompt(k).await);
        }
        if k == "last_step" {
            let mut guard = session.lock().await;
            return guard.render_last_llm_step_record().await;
        }
        if k.starts_with("last_steps") {
            let max_pull = parse_pull_limit_from_key(k, "last_steps", DEFAULT_LAST_STEPS_MAX_PULL);
            let options = LLMStepPromptRenderOptions {
                max_render_steps: max_pull,
                recent_detail_steps: max_pull,
                ..LLMStepPromptRenderOptions::default()
            };
            let mut guard = session.lock().await;
            return guard
                .render_llm_step_records_prompt(Some(&options))
                .await
                .map(|text| clean_optional_text(Some(text.as_str())));
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
            debug!(
                "agent_env.new_msg_pull: session={} sub_id={} count={}",
                session_id,
                kmsg_sub_id,
                new_msgs.len()
            );

            if let Some(last_msg) = new_msgs.last() {
                let mut guard = session.lock().await;
                // save cursor for ack after process
                guard.just_readed_input_msg = new_msgs.iter().map(|r| r.payload.clone()).collect();
                guard.msg_kmsgqueue_curosr = last_msg.index;
            }

            return Ok(render_new_msgs_from_kmsgqueue(new_msgs.as_slice()).await);
        }

        if k.starts_with("new_event") {
            let max_pull = parse_pull_limit_from_key(k, "new_event", DEFAULT_NEW_EVENT_MAX_PULL);
            let kmsg_sub_id = AIAgent::get_session_kmsgqueue_sub_id(
                owner_agent.as_str(),
                session_id.as_str(),
                InputQueueKind::Event,
            );
            let max_pull_u32 = (max_pull.min(u32::MAX as usize)) as u32;
            let new_events =
                AgentSession::pull_new_msg_from_kmsgqueue(kmsg_sub_id.as_str(), max_pull_u32).await;

            if new_events.is_err() {
                warn!(
                    "agent_env.load_value_from_session pull_new_event failed: session={} sub={} err={}",
                    session_id,
                    kmsg_sub_id,
                    new_events.err().unwrap()
                );
                return Ok(None);
            }

            let new_events = new_events.unwrap();
            if new_events.is_empty() {
                return Ok(None);
            }
            debug!(
                "agent_env.new_event_pull: session={} sub_id={} count={}",
                session_id,
                kmsg_sub_id,
                new_events.len()
            );

            if let Some(last_msg) = new_events.last() {
                let mut guard = session.lock().await;
                guard.just_readed_input_event =
                    new_events.iter().map(|r| r.payload.clone()).collect();
                guard.event_kmsgqueue_curosr = last_msg.index;
            }

            return Ok(render_new_events_from_kmsgqueue(new_events.as_slice()));
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

        if k.starts_with("local_workspace_list") || k.starts_with("workspace_list") {
            // k format: local_workspace_list / local_workspace_list.$num
            // or workspace_list / workspace_list.$num
            let prefix = if k.starts_with("workspace_list") {
                "workspace_list"
            } else {
                "local_workspace_list"
            };
            let max_pull =
                parse_pull_limit_from_key(k, prefix, DEFAULT_LOCAL_WORKSPACE_LIST_MAX_PULL);
            return Ok(render_recent_local_workspaces_from_disk(
                workspace_info.as_ref(),
                &session_cwd,
                max_pull,
            )
            .await);
        }
        if k == "current_todo"
            || k == "workspace_current_todo_id"
            || k == "workspace_current_todo"
            || k == "workspace_next_ready_todo"
            || k == "workspace.todolist.next_ready_todo"
        {
            let value_kind = if k == "current_todo" || k == "workspace_current_todo_id" {
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
                if k == "workspace_current_todo_id" {
                    return Ok(resolve_workspace_info_text(workspace_info, "current_todo")
                        .or_else(|| resolve_workspace_info_text(workspace_info, k)));
                }
                return Ok(resolve_workspace_info_text(workspace_info, k));
            }
            return Ok(None);
        }

        if k == "workspace_todolist" {
            let workspace_id = resolve_session_workspace_id(
                local_workspace_id.as_deref(),
                workspace_info.as_ref(),
            );
            let todo_db_path = resolve_todo_db_path(
                local_workspace_id.as_deref(),
                workspace_info.as_ref(),
                &session_cwd,
            );

            if let (Some(workspace_id), Some(todo_db_path)) = (workspace_id, todo_db_path) {
                if todo_db_path.is_file() {
                    match load_workspace_todo_list_text(
                        todo_db_path.clone(),
                        workspace_id.clone(),
                        DEFAULT_WORKSPACE_TODO_LIST_MAX_ITEMS,
                    )
                    .await
                    {
                        Ok(Some(value)) => return Ok(Some(value)),
                        Ok(None) => {}
                        Err(err) => {
                            warn!(
                                "agent_env.load_value_from_session workspace_todolist query failed: session={} workspace={} db={} err={}",
                                session_id,
                                workspace_id,
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

        if let Some(todo_ref_raw) = k
            .strip_prefix("workspace_todolist.")
            .or_else(|| k.strip_prefix("workspace.todolist."))
        {
            let todo_ref = todo_ref_raw.trim();
            if !todo_ref.is_empty() && todo_ref != "next_ready_todo" {
                let workspace_id = resolve_session_workspace_id(
                    local_workspace_id.as_deref(),
                    workspace_info.as_ref(),
                );
                let todo_db_path = resolve_todo_db_path(
                    local_workspace_id.as_deref(),
                    workspace_info.as_ref(),
                    &session_cwd,
                );

                if let (Some(workspace_id), Some(todo_db_path)) = (workspace_id, todo_db_path) {
                    if todo_db_path.is_file() {
                        match load_session_todo_text_by_ref(
                            todo_db_path.clone(),
                            workspace_id.clone(),
                            session_id.clone(),
                            todo_ref.to_string(),
                        )
                        .await
                        {
                            Ok(Some(value)) => return Ok(Some(value)),
                            Ok(None) => {}
                            Err(err) => {
                                warn!(
                                    "agent_env.load_value_from_session todo_ref query failed: session={} workspace={} key={} db={} err={}",
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
        }

        if k.starts_with("workspace.") {
            return Ok(workspace_info
                .as_ref()
                .and_then(|ws| resolve_workspace_info_text(ws, k)));
        }

        let workspace_root = resolve_session_workspace_root(workspace_info.as_ref(), &session_cwd)
            .unwrap_or(std::env::current_dir().map_err(|err| {
                AgentToolError::ExecFailed(format!("read current_dir failed: {err}"))
            })?);
        let cwd_root = non_empty_path(&session_cwd).unwrap_or_else(|| workspace_root.clone());
        let agent_root = resolve_agent_env_root(workspace_info.as_ref(), &session_cwd)
            .or_else(|| {
                resolve_agent_env_root_from_local_workspace_hint(
                    local_workspace_id.as_deref(),
                    workspace_info.as_ref(),
                    &session_cwd,
                )
            })
            .unwrap_or_else(|| workspace_root.clone());
        let session_root =
            resolve_current_session_dir(&session_root_dir, &session_cwd, session_id.as_str()).await;

        if k.starts_with("$agent_root/") {
            let rel_path = &k["$agent_root/".len()..];
            return resolve_path_from_root(agent_root.as_path(), rel_path);
        }
        if k.starts_with("$session_root/") {
            let rel_path = &k["$session_root/".len()..];
            if let Some(session_root) = session_root.as_ref() {
                return resolve_path_from_root(session_root.as_path(), rel_path);
            }
            return Ok(None);
        }
        if k == "workspace_root" || k == "$workspace_root" {
            return Ok(Some(workspace_root.to_string_lossy().to_string()));
        }
        if k.starts_with("$workspace_root/") {
            let rel_path = &k["$workspace_root/".len()..];
            return resolve_path_from_root(workspace_root.as_path(), rel_path);
        }
        if k.starts_with("$workspace/") {
            let rel_path = &k["$workspace/".len()..];
            return resolve_path_from_root(workspace_root.as_path(), rel_path);
        }
        if k.starts_with("$cwd/") {
            let rel_path = &k["$cwd/".len()..];
            return resolve_path_from_root(cwd_root.as_path(), rel_path);
        }

        Ok(None)
    }

    pub async fn render_prompt(
        input: &str,
        env_context: &HashMap<String, Json>,
        session: Arc<Mutex<AgentSession>>,
    ) -> Result<AgentTemplateRenderResult, AgentToolError> {
        let prepared_input = prepare_prompt_template(input);
        let session_clone = session.clone();
        Self::render_text_template(
            prepared_input.as_str(),
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
            "workspace" => self.agent_env_root().to_path_buf(),
            "cwd" => ctx
                .cwd_path
                .clone()
                .unwrap_or_else(|| self.agent_env_root().to_path_buf()),
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

async fn preprocess_text_template_pass<F, Fut>(
    input: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
    render_ctx: &mut Map<String, Json>,
    resolved_vars: &mut HashMap<String, bool>,
) -> Result<(String, bool, u32, u32, u32, u32, u32, u32), AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    const ENV_TOKEN_OPEN: &str = "__OPENDAN_ENV(";
    const CONTENT_TOKEN_OPEN: &str = "__OPENDAN_CONTENT(";
    const EXEC_TOKEN_OPEN: &str = "__OPENDAN_EXEC(";
    const VAR_TOKEN_OPEN: &str = "__OPENDAN_VAR(";
    const TOKEN_PREFIX: &str = "__OPENDAN_";

    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut changed = false;
    let mut env_expanded = 0u32;
    let mut env_not_found = 0u32;
    let mut content_loaded = 0u32;
    let mut content_failed = 0u32;
    let mut var_registered = 0u32;
    let mut var_failed = 0u32;

    while let Some(start) = input[cursor..].find(TOKEN_PREFIX).map(|idx| cursor + idx) {
        output.push_str(&input[cursor..start]);

        if input[start..].starts_with(ENV_TOKEN_OPEN) {
            let arg_start = start + ENV_TOKEN_OPEN.len();
            let Some(arg_end) = input[arg_start..].find(")__").map(|idx| arg_start + idx) else {
                return Err(AgentToolError::ExecFailed(
                    "render text template failed: malformed __OPENDAN_ENV directive".to_string(),
                ));
            };
            let expr = input[arg_start..arg_end].trim();
            if !expr.starts_with('$') {
                return Err(AgentToolError::ExecFailed(format!(
                    "render text template failed: __OPENDAN_ENV expects a dynamic variable expression, got `{expr}`"
                )));
            }

            let value = resolve_dynamic_value(expr, load_value, env_context).await?;
            if value.is_some() {
                env_expanded = env_expanded.saturating_add(1);
            } else {
                env_not_found = env_not_found.saturating_add(1);
            }
            output.push_str(
                &value
                    .as_ref()
                    .and_then(json_value_to_compact_text)
                    .unwrap_or_default(),
            );
            cursor = arg_end + 3;
            changed = true;
            continue;
        }

        if input[start..].starts_with(CONTENT_TOKEN_OPEN) {
            let arg_start = start + CONTENT_TOKEN_OPEN.len();
            let Some(arg_end) = input[arg_start..].find(")__").map(|idx| arg_start + idx) else {
                return Err(AgentToolError::ExecFailed(
                    "render text template failed: malformed __OPENDAN_CONTENT directive"
                        .to_string(),
                ));
            };
            let path_arg = input[arg_start..arg_end].trim();
            match load_text_from_content_arg(path_arg, load_value, env_context).await? {
                ContentLoadResult::Loaded(content) => {
                    content_loaded = content_loaded.saturating_add(1);
                    output.push_str(content.as_str());
                }
                ContentLoadResult::Missing(absolute_path) => {
                    content_failed = content_failed.saturating_add(1);
                    warn!(
                        "agent_env.render_text_template missing __OPENDAN_CONTENT file, substituting empty text: path={}",
                        absolute_path.display()
                    );
                }
            }
            cursor = arg_end + 3;
            changed = true;
            continue;
        }

        if input[start..].starts_with(EXEC_TOKEN_OPEN) {
            let arg_start = start + EXEC_TOKEN_OPEN.len();
            let Some(arg_end) = input[arg_start..].find(")__").map(|idx| arg_start + idx) else {
                return Err(AgentToolError::ExecFailed(
                    "render text template failed: malformed __OPENDAN_EXEC directive".to_string(),
                ));
            };
            let command_arg = input[arg_start..arg_end].trim();
            let output_text =
                execute_text_template_command(command_arg, load_value, env_context).await?;
            output.push_str(output_text.as_str());
            cursor = arg_end + 3;
            changed = true;
            continue;
        }

        if input[start..].starts_with(VAR_TOKEN_OPEN) {
            let arg_start = start + VAR_TOKEN_OPEN.len();
            let Some(arg_end) = input[arg_start..].find(')').map(|idx| arg_start + idx) else {
                return Err(AgentToolError::ExecFailed(
                    "render text template failed: malformed __OPENDAN_VAR directive".to_string(),
                ));
            };
            let raw_args = input[arg_start..arg_end].trim();
            let Some((var_name_raw, expr_raw)) = raw_args.split_once(',') else {
                return Err(AgentToolError::ExecFailed(
                    "render text template failed: __OPENDAN_VAR requires `var_name, $expr`"
                        .to_string(),
                ));
            };

            let var_name = var_name_raw.trim();
            if !is_variable_name(var_name) {
                return Err(AgentToolError::ExecFailed(format!(
                    "render text template failed: invalid __OPENDAN_VAR name `{var_name}`"
                )));
            }

            let expr = expr_raw.trim();
            if !expr.starts_with('$') {
                return Err(AgentToolError::ExecFailed(format!(
                    "render text template failed: __OPENDAN_VAR expects a dynamic variable expression, got `{expr}`"
                )));
            }

            let value = resolve_dynamic_value(expr, load_value, env_context).await?;
            let resolved_entry = resolved_vars.entry(var_name.to_string()).or_insert(false);
            if let Some(value) = value {
                render_ctx.insert(var_name.to_string(), value);
                var_registered = var_registered.saturating_add(1);
                *resolved_entry = true;
            } else {
                render_ctx.insert(var_name.to_string(), Json::String(String::new()));
                var_failed = var_failed.saturating_add(1);
            }
            cursor = arg_end + 1;
            changed = true;
            continue;
        }

        return Err(AgentToolError::ExecFailed(format!(
            "render text template failed: unsupported OpenDAN directive near `{}`",
            truncate_chars(&input[start..], 64)
        )));
    }

    if cursor < input.len() {
        output.push_str(&input[cursor..]);
    }

    Ok((
        output,
        changed,
        env_expanded,
        env_not_found,
        content_loaded,
        content_failed,
        var_registered,
        var_failed,
    ))
}

async fn resolve_dynamic_value<F, Fut>(
    expr: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
) -> Result<Option<Json>, AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    let trimmed = expr.trim();
    let Some(key) = trimmed.strip_prefix('$').map(str::trim) else {
        return Err(AgentToolError::ExecFailed(format!(
            "dynamic variable expression must start with `$`: {trimmed}"
        )));
    };

    if key.is_empty() {
        return Ok(None);
    }

    if let Some(value) = resolve_env_context_value(env_context, key) {
        return Ok(Some(value.clone()));
    }

    if let Some(value) = clean_optional_text(load_value(key).await?.as_deref()) {
        return Ok(Some(parse_loaded_dynamic_value(value.as_str())));
    }

    if let Some(value) = clean_optional_text(load_value(trimmed).await?.as_deref()) {
        return Ok(Some(parse_loaded_dynamic_value(value.as_str())));
    }

    Ok(None)
}

fn parse_loaded_dynamic_value(value: &str) -> Json {
    let trimmed = value.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Json>(trimmed) {
            return parsed;
        }
    }
    Json::String(trimmed.to_string())
}

enum ContentLoadResult {
    Loaded(String),
    Missing(PathBuf),
}

async fn load_text_from_content_arg<F, Fut>(
    path_arg: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
) -> Result<ContentLoadResult, AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    let absolute_path = resolve_content_absolute_path(path_arg, load_value, env_context).await?;
    load_text_from_absolute_path(&absolute_path).await
}

async fn resolve_content_absolute_path<F, Fut>(
    path_arg: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
) -> Result<PathBuf, AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    let raw_path = if path_arg.starts_with('$') {
        resolve_dynamic_value(path_arg, load_value, env_context)
            .await?
            .as_ref()
            .and_then(json_value_to_compact_text)
            .ok_or_else(|| {
                AgentToolError::ExecFailed(format!(
                    "render text template failed: __OPENDAN_CONTENT path expression `{path_arg}` resolved to empty"
                ))
            })?
    } else if path_arg.starts_with('/') {
        path_arg.trim().to_string()
    } else {
        return Err(AgentToolError::ExecFailed(format!(
            "render text template failed: __OPENDAN_CONTENT expects a dynamic variable or absolute path, got `{path_arg}`"
        )));
    };

    let expanded_path = expand_system_env_vars(raw_path.as_str());
    let absolute_path = PathBuf::from(expanded_path.trim());
    if !absolute_path.is_absolute() {
        return Err(AgentToolError::ExecFailed(format!(
            "render text template failed: __OPENDAN_CONTENT path must be absolute after env expansion, got `{}`",
            expanded_path.trim()
        )));
    }

    Ok(absolute_path)
}

async fn load_text_from_absolute_path(
    absolute_path: &Path,
) -> Result<ContentLoadResult, AgentToolError> {
    let bytes = match fs::read(absolute_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(ContentLoadResult::Missing(absolute_path.to_path_buf()));
        }
        Err(err) => {
            return Err(AgentToolError::ExecFailed(format!(
                "render text template failed: read content `{}` failed: {err}",
                absolute_path.display()
            )));
        }
    };
    let content = String::from_utf8(bytes).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "render text template failed: decode content `{}` failed: {err}",
            absolute_path.display()
        ))
    })?;

    Ok(ContentLoadResult::Loaded(truncate_utf8(
        &content,
        MAX_INCLUDE_BYTES,
    )))
}

async fn execute_text_template_command<F, Fut>(
    command_arg: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
) -> Result<String, AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    let command = parse_exec_command_arg(command_arg)?;
    if command.is_empty() {
        return Err(AgentToolError::ExecFailed(
            "render text template failed: __OPENDAN_EXEC command is empty".to_string(),
        ));
    }

    let expanded_command =
        expand_exec_command_dynamic_values(command, load_value, env_context).await?;
    let output = timeout(
        Duration::from_millis(TEMPLATE_EXEC_TIMEOUT_MS),
        Command::new("sh")
            .arg("-lc")
            .arg(expanded_command.as_str())
            .output(),
    )
    .await
    .map_err(|_| {
        AgentToolError::ExecFailed(format!(
            "render text template failed: __OPENDAN_EXEC timed out after {}ms: `{}`",
            TEMPLATE_EXEC_TIMEOUT_MS,
            truncate_chars(expanded_command.as_str(), 160)
        ))
    })?
    .map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "render text template failed: __OPENDAN_EXEC spawn failed: command=`{}` err={err}",
            truncate_chars(expanded_command.as_str(), 160)
        ))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = truncate_chars(stderr.trim(), 200);
        let exit_code = output.status.code().unwrap_or_default();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!(" stderr={stderr}")
        };
        return Err(AgentToolError::ExecFailed(format!(
            "render text template failed: __OPENDAN_EXEC command failed: exit_code={} command=`{}`{}",
            exit_code,
            truncate_chars(expanded_command.as_str(), 160),
            detail
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(truncate_utf8(stdout.as_ref(), MAX_INCLUDE_BYTES))
}

fn parse_exec_command_arg(command_arg: &str) -> Result<&str, AgentToolError> {
    let trimmed = command_arg.trim();
    if trimmed.is_empty() {
        return Ok(trimmed);
    }

    let first = trimmed.chars().next().unwrap_or_default();
    let last = trimmed.chars().last().unwrap_or_default();
    if (first == '"' || first == '\'') && first != last {
        return Err(AgentToolError::ExecFailed(format!(
            "render text template failed: malformed __OPENDAN_EXEC command `{}`",
            truncate_chars(trimmed, 160)
        )));
    }
    if (first == '"' || first == '\'') && trimmed.len() >= 2 {
        return Ok(&trimmed[1..trimmed.len() - 1]);
    }
    Ok(trimmed)
}

async fn expand_exec_command_dynamic_values<F, Fut>(
    command: &str,
    load_value: &F,
    env_context: &HashMap<String, Json>,
) -> Result<String, AgentToolError>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<Option<String>, AgentToolError>>,
{
    let mut output = String::with_capacity(command.len());
    let mut cursor = 0usize;

    while cursor < command.len() {
        let next_dollar = command[cursor..]
            .find('$')
            .map(|idx| cursor + idx)
            .unwrap_or(command.len());
        output.push_str(&command[cursor..next_dollar]);
        if next_dollar >= command.len() {
            break;
        }

        let prev_char = command[..next_dollar].chars().last();
        if matches!(prev_char, Some('\\')) {
            output.push('$');
            cursor = next_dollar + 1;
            continue;
        }

        let token_len = command[next_dollar..]
            .chars()
            .take_while(|ch| {
                *ch == '$' || ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-')
            })
            .map(char::len_utf8)
            .sum::<usize>();
        if token_len <= 1 {
            output.push('$');
            cursor = next_dollar + 1;
            continue;
        }

        let expr = &command[next_dollar..next_dollar + token_len];
        let resolved = resolve_dynamic_value(expr, load_value, env_context).await?;
        if let Some(text) = resolved.as_ref().and_then(json_value_to_compact_text) {
            output.push_str(text.as_str());
        } else {
            output.push_str(expr);
        }
        cursor = next_dollar + token_len;
    }

    Ok(output)
}

fn expand_system_env_vars(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut idx = 0usize;

    while idx < chars.len() {
        if chars[idx] != '$' {
            output.push(chars[idx]);
            idx += 1;
            continue;
        }

        if idx + 1 >= chars.len() {
            output.push('$');
            idx += 1;
            continue;
        }

        if chars[idx + 1] == '{' {
            let mut end = idx + 2;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end >= chars.len() {
                output.push('$');
                idx += 1;
                continue;
            }

            let name = chars[idx + 2..end].iter().collect::<String>();
            output.push_str(std::env::var(name.as_str()).unwrap_or_default().as_str());
            idx = end + 1;
            continue;
        }

        let mut end = idx + 1;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if end == idx + 1 {
            output.push('$');
            idx += 1;
            continue;
        }

        let name = chars[idx + 1..end].iter().collect::<String>();
        output.push_str(std::env::var(name.as_str()).unwrap_or_default().as_str());
        idx = end;
    }

    output
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

    let lines = records
        .into_iter()
        .take(max_pull)
        .map(|record| {
            // let title = clean_optional_text(Some(record.title.as_str()))
            //     .unwrap_or_else(|| format!("Session {}", record.session_id));
            let activity_ms = record
                .last_activity_ms
                .max(record.updated_at_ms)
                .max(record.created_at_ms);
            format!(
                "{} | {}  ",
                record.session_id,
                format_compact_timestamp(activity_ms)
            )
        })
        .collect::<Vec<_>>();

    let rendered = lines.join("\n");
    clean_optional_text(Some(rendered.as_str()))
}

async fn load_owner_value_for_prompt(key: &str) -> Option<String> {
    let runtime = match get_buckyos_api_runtime() {
        Ok(runtime) => runtime,
        Err(err) => {
            debug!("agent_env.load_owner runtime_unavailable: err={}", err);
            return None;
        }
    };

    let owner_did = resolve_owner_did_from_runtime(&runtime)?;
    let owner_user_id = runtime
        .get_owner_user_id()
        .and_then(|value| normalize_optional_text(Some(value.as_str())));

    let msg_center = match runtime.get_msg_center_client().await {
        Ok(client) => client,
        Err(err) => {
            warn!("agent_env.load_owner msg_center_unavailable: err={}", err);
            let value = build_owner_json_value(&owner_did, owner_user_id.as_deref(), None);
            return resolve_owner_json_value(key, &value);
        }
    };

    let contact = match msg_center
        .get_contact(owner_did.clone(), Some(owner_did.clone()))
        .await
    {
        Ok(contact) => contact,
        Err(err) => {
            warn!(
                "agent_env.load_owner get_contact_failed: owner={} err={}",
                owner_did.to_string(),
                err
            );
            let value = build_owner_json_value(&owner_did, owner_user_id.as_deref(), None);
            return resolve_owner_json_value(key, &value);
        }
    };

    let value = build_owner_json_value(&owner_did, owner_user_id.as_deref(), contact.as_ref());
    resolve_owner_json_value(key, &value)
}

fn resolve_owner_did_from_runtime(runtime: &buckyos_api::BuckyOSRuntime) -> Option<DID> {
    if let Some(zone_config) = runtime.get_zone_config() {
        if zone_config.owner.is_valid() {
            return Some(zone_config.owner.clone());
        }
    }

    runtime
        .get_owner_user_id()
        .and_then(|owner_id| normalize_optional_text(Some(owner_id.as_str())))
        .map(|owner_id| DID::new("bns", owner_id.as_str()))
}

fn resolve_owner_json_value(key: &str, value: &Json) -> Option<String> {
    if key == "owner" {
        return json_value_to_compact_text(value);
    }

    key.strip_prefix("owner.")
        .and_then(|path| resolve_json_path(value, path))
        .and_then(json_value_to_compact_text)
}

fn build_owner_json_value(
    owner_did: &DID,
    owner_user_id: Option<&str>,
    contact: Option<&Contact>,
) -> Json {
    let user_id = normalize_optional_text(owner_user_id).unwrap_or_else(|| owner_did.id.clone());
    let show_name = contact
        .and_then(|contact| normalize_optional_text(Some(contact.name.as_str())))
        .unwrap_or_else(|| user_id.clone());

    let contact_value = match contact {
        Some(contact) => {
            let bindings = contact
                .bindings
                .iter()
                .map(|binding| {
                    serde_json::json!({
                        "platform": binding.platform,
                        "account_id": binding.account_id,
                        "display_id": binding.display_id,
                        "tunnel_id": binding.tunnel_id,
                        "last_active_at": binding.last_active_at,
                        "meta": binding.meta,
                    })
                })
                .collect::<Vec<_>>();

            serde_json::json!({
                "did": contact.did.to_string(),
                "name": contact.name,
                "avatar": contact.avatar,
                "note": contact.note,
                "source": contact.source,
                "is_verified": contact.is_verified,
                "access_level": contact.access_level,
                "groups": contact.groups,
                "tags": contact.tags,
                "bindings": bindings,
            })
        }
        None => serde_json::json!({
            "did": owner_did.to_string(),
            "name": show_name,
            "avatar": null,
            "note": null,
            "groups": [],
            "tags": [],
            "bindings": [],
        }),
    };

    serde_json::json!({
        "user_id": user_id,
        "did": owner_did.to_string(),
        "name": show_name,
        "show_name": show_name,
        "contact": contact_value,
    })
}

async fn render_recent_local_workspaces_from_disk(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
    max_pull: usize,
) -> Option<String> {
    if max_pull == 0 {
        return None;
    }

    let root = resolve_agent_env_root(workspace_info, session_cwd)?;
    let index_path = root.join(WORKSHOP_INDEX_FILE_NAME);
    if !fs::metadata(&index_path)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false)
    {
        return None;
    }

    let local_root = root.join("workspaces");
    if !fs::metadata(&local_root)
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
    {
        return None;
    }

    let Ok(raw) = fs::read_to_string(&index_path).await else {
        return None;
    };
    let Ok(index) = serde_json::from_str::<WorkshopIndex>(&raw) else {
        return None;
    };
    let records = index.workspaces;

    if records.is_empty() {
        return None;
    }

    let mut local_records = records
        .into_iter()
        .filter(|record| record.workspace_type == WorkspaceType::Local)
        .collect::<Vec<_>>();

    if local_records.is_empty() {
        return None;
    }

    local_records.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
            .then_with(|| a.workspace_id.cmp(&b.workspace_id))
    });

    let lines = local_records
        .into_iter()
        .take(max_pull)
        .filter_map(|record| {
            let workspace_id = clean_optional_text(Some(record.workspace_id.as_str()))?;
            let summary = clean_optional_text(Some(record.name.as_str()))
                .map(|value| collapse_whitespace(value.as_str()))
                .map(|value| truncate_chars(value.as_str(), 200))
                .unwrap_or_else(|| "local workspace".to_string());
            Some(format!("\n- ${workspace_id} \n  {summary}\n"))
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return None;
    }
    clean_optional_text(Some(lines.join("\n").as_str()))
}

async fn resolve_current_session_dir(
    session_root_dir: &Path,
    session_cwd: &Path,
    current_session_id: &str,
) -> Option<PathBuf> {
    let session_id = clean_optional_text(Some(current_session_id))?;
    if let Some(root) = non_empty_path(session_root_dir) {
        return Some(root.join(session_id));
    }
    resolve_session_root_from_cwd(session_cwd, current_session_id)
        .await
        .map(|root| root.join(current_session_id.trim()))
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
            let session_root = ancestor.join("sessions");
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

fn parse_event_from_kmsg_payload(payload: &[u8]) -> Option<(String, Option<Json>)> {
    let input_item = serde_json::from_slice::<SessionInputItem>(payload).ok()?;
    let event_id = input_item.event_id?;
    Some((event_id, input_item.event_data))
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
            if let Ok(raw_text) = String::from_utf8(message.payload.clone()) {
                let content = normalize_multiline_text(raw_text.as_str());
                if !content.is_empty() {
                    lines.push(format!(
                        "unknown [{}]\n{}",
                        format_timestamp(message.created_at),
                        content
                    ));
                    continue;
                }
            }
            warn!(
                "agent_env.render_new_msgs invalid payload: expected SessionInputItem.msg or MsgRecord"
            );
            continue;
        };

        let mut msg_obj = None::<MsgObject>;
        if let Some(store) = named_store.as_ref() {
            let msg_id = msg_record.msg_id.clone();
            match store.get_object(&msg_id).await {
                Ok(msg_json) => match serde_json::from_str::<MsgObject>(&msg_json) {
                    Ok(value) => msg_obj = Some(value),
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

        lines.push(render_human_readable_msg_line(
            &msg_record,
            msg_obj.as_ref(),
        ));
    }
    if lines.is_empty() {
        return None;
    }
    clean_optional_text(Some(lines.join("\n\n").as_str()))
}

fn render_new_events_from_kmsgqueue(messages: &[Message]) -> Option<String> {
    if messages.is_empty() {
        return None;
    }

    let mut lines = Vec::<String>::new();
    for message in messages {
        let Some((event_id, event_data)) = parse_event_from_kmsg_payload(&message.payload) else {
            if let Ok(raw_text) = String::from_utf8(message.payload.clone()) {
                let content = normalize_multiline_text(raw_text.as_str());
                if !content.is_empty() {
                    let mut event = Map::<String, Json>::new();
                    let mut event_data = Map::<String, Json>::new();
                    event_data.insert("raw".to_string(), Json::String(content));
                    event.insert("eventid".to_string(), Json::String("unknown".to_string()));
                    event.insert("eventdata".to_string(), Json::Object(event_data));
                    if let Ok(text) = serde_json::to_string_pretty(&Json::Object(event)) {
                        lines.push(text);
                    }
                }
            }
            continue;
        };

        let mut event = Map::<String, Json>::new();
        event.insert("eventid".to_string(), Json::String(event_id));
        if let Some(event_data) = event_data {
            event.insert("eventdata".to_string(), event_data);
        }

        if let Ok(text) = serde_json::to_string_pretty(&Json::Object(event)) {
            lines.push(text);
        }
    }

    if lines.is_empty() {
        return None;
    }
    clean_optional_text(Some(lines.join("\n\n").as_str()))
}

fn render_human_readable_msg_line(record: &MsgRecord, msg_obj: Option<&MsgObject>) -> String {
    let from = record
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            msg_obj
                .map(|msg| msg.from.to_raw_host_name())
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| record.from.to_raw_host_name());
    let time_ms = msg_obj
        .map(|msg| msg.created_at_ms)
        .unwrap_or(record.updated_at_ms.max(record.created_at_ms));
    let content = msg_obj
        .map(|msg| msg.content.content.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_multiline_text)
        .unwrap_or_else(|| format!("msg_id={} state={:?}", record.msg_id, record.state));
    format!("{} [{}]\n{}", from, format_timestamp(time_ms), content)
}

fn format_timestamp(timestamp_ms: u64) -> String {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
    let dt = DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now);
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn format_compact_timestamp(timestamp_ms: u64) -> String {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
    let dt = DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now);
    format!(
        "{}-{}-{} {:02}:{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

fn normalize_multiline_text(input: &str) -> String {
    let mut lines = Vec::<String>::new();
    let mut last_blank = false;
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if last_blank {
                continue;
            }
            lines.push(String::new());
            last_blank = true;
            continue;
        }
        lines.push(line.to_string());
        last_blank = false;
    }
    let merged = lines.join("\n");
    if merged.trim().is_empty() {
        String::new()
    } else {
        merged.trim().to_string()
    }
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

async fn load_session_todo_text_by_ref(
    db_path: PathBuf,
    workspace_id: String,
    session_id: String,
    todo_ref: String,
) -> Result<Option<String>, AgentToolError> {
    task::spawn_blocking(move || {
        let conn = Connection::open(&db_path).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open todo db `{}` failed: {err}",
                db_path.display()
            ))
        })?;
        get_session_todo_text_by_ref(&conn, &workspace_id, &session_id, &todo_ref)
    })
    .await
    .map_err(|err| AgentToolError::ExecFailed(format!("query session todo join failed: {err}")))?
}

async fn load_workspace_todo_list_text(
    db_path: PathBuf,
    workspace_id: String,
    max_items: usize,
) -> Result<Option<String>, AgentToolError> {
    task::spawn_blocking(move || {
        let conn = Connection::open(&db_path).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open todo db `{}` failed: {err}",
                db_path.display()
            ))
        })?;

        let version_key = format!("version:{workspace_id}");
        let version = conn
            .query_row(
                "SELECT value FROM todo_meta WHERE key = ?1 LIMIT 1",
                params![version_key],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);

        let mut stmt = conn
            .prepare(
                "SELECT todo_code, status, assignee_did, priority, title
                 FROM todo_items
                 WHERE workspace_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("prepare workspace todo list failed: {err}"))
            })?;
        let mut rows = stmt
            .query(params![workspace_id.as_str(), max_items as i64])
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("query workspace todo list failed: {err}"))
            })?;

        let mut lines = vec![format!("Workspace Todo ({workspace_id}, v{version})")];
        let mut count = 0usize;
        while let Some(row) = rows.next().map_err(|err| {
            AgentToolError::ExecFailed(format!("iterate workspace todo list failed: {err}"))
        })? {
            let todo_code = row.get::<_, String>(0).map_err(|err| {
                AgentToolError::ExecFailed(format!("decode workspace todo_code failed: {err}"))
            })?;
            let status = row.get::<_, String>(1).map_err(|err| {
                AgentToolError::ExecFailed(format!("decode workspace todo status failed: {err}"))
            })?;
            let assignee = row
                .get::<_, Option<String>>(2)
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "decode workspace todo assignee failed: {err}"
                    ))
                })?
                .unwrap_or_else(|| "-".to_string());
            let priority = row
                .get::<_, Option<i64>>(3)
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "decode workspace todo priority failed: {err}"
                    ))
                })?
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let title = row.get::<_, String>(4).map_err(|err| {
                AgentToolError::ExecFailed(format!("decode workspace todo title failed: {err}"))
            })?;

            lines.push(format!(
                "- {} [{}] assignee={} p={} {}",
                todo_code,
                status,
                assignee,
                priority,
                truncate_chars(collapse_whitespace(title.as_str()).as_str(), 200)
            ));
            count = count.saturating_add(1);
        }

        if count == 0 {
            lines.push("- No todo items available.".to_string());
        }

        let rendered = lines.join("\n");
        Ok(clean_optional_text(Some(rendered.as_str())))
    })
    .await
    .map_err(|err| {
        AgentToolError::ExecFailed(format!("query workspace todo list join failed: {err}"))
    })?
}

fn resolve_session_workspace_id(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
) -> Option<String> {
    normalize_optional_text(local_workspace_id)
        .or_else(|| {
            resolve_workspace_binding_ref(workspace_info)
                .and_then(|binding| binding.normalized_local_workspace_id())
        })
        .or_else(|| resolve_bound_workspace_id(workspace_info))
}

fn resolve_todo_db_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    if let Some(local_workspace_path) =
        resolve_default_local_workspace_path(local_workspace_id, workspace_info, session_cwd)
    {
        if !local_workspace_path.is_dir() {
            return None;
        }
    }

    resolve_agent_env_root_from_local_workspace_hint(
        local_workspace_id,
        workspace_info,
        session_cwd,
    )
    .map(|root| root.join(WORKSHOP_TODO_DB_REL_PATH))
    .filter(|path| path.is_file())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn resolve_workspace_info_text(workspace_info: &Json, key: &str) -> Option<String> {
    resolve_json_path(workspace_info, key).and_then(json_value_to_compact_text)
}

fn resolve_path_from_root(root: &Path, rel_path: &str) -> Result<Option<String>, AgentToolError> {
    let rel_path = rel_path.trim();
    if rel_path.is_empty() || !is_safe_relative_path(rel_path) {
        warn!("agent_env.render skip unsafe path ref: rel_path={rel_path}");
        return Ok(None);
    }

    let absolute_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };

    Ok(Some(
        absolute_root.join(rel_path).to_string_lossy().to_string(),
    ))
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

fn prepare_prompt_template(template: &str) -> String {
    if template.trim().is_empty() {
        return String::new();
    }

    let escaped = escape_template_literals(template);
    let declared_vars = collect_declared_prompt_vars(escaped.as_str());
    let placeholder_vars = collect_plain_placeholder_vars(escaped.as_str());
    let mut auto_vars = Vec::<String>::new();

    for var_name in placeholder_vars {
        if !declared_vars.contains(var_name.as_str()) {
            auto_vars.push(var_name);
        }
    }

    if auto_vars.is_empty() {
        return template.to_string();
    }

    let prologue = auto_vars
        .into_iter()
        .map(|var_name| format!("__OPENDAN_VAR({0}, ${0})", var_name))
        .collect::<Vec<_>>()
        .join("");
    format!("{prologue}{template}")
}

fn collect_declared_prompt_vars(template: &str) -> HashSet<String> {
    const VAR_TOKEN_OPEN: &str = "__OPENDAN_VAR(";

    let mut declared = HashSet::<String>::new();
    let mut cursor = 0usize;
    while let Some(start) = template[cursor..]
        .find(VAR_TOKEN_OPEN)
        .map(|idx| cursor + idx)
    {
        let arg_start = start + VAR_TOKEN_OPEN.len();
        let Some(arg_end) = template[arg_start..].find(')').map(|idx| arg_start + idx) else {
            break;
        };
        let raw_args = template[arg_start..arg_end].trim();
        if let Some((var_name_raw, _expr_raw)) = raw_args.split_once(',') {
            let var_name = var_name_raw.trim();
            if is_variable_name(var_name) {
                declared.insert(var_name.to_string());
            }
        }
        cursor = arg_end.saturating_add(1);
    }
    declared
}

fn collect_plain_placeholder_vars(template: &str) -> Vec<String> {
    let mut vars = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let mut cursor = 0usize;

    while let Some(open_pos) = template[cursor..].find("{{").map(|idx| cursor + idx) {
        let content_start = open_pos + 2;
        let Some(close_pos) = template[content_start..]
            .find("}}")
            .map(|idx| content_start + idx)
        else {
            break;
        };
        let placeholder = template[content_start..close_pos].trim();
        if is_variable_name(placeholder) {
            let name = placeholder.to_string();
            if seen.insert(name.clone()) {
                vars.push(name);
            }
        }
        cursor = close_pos + 2;
    }

    vars
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
    use crate::agent_tool::{AgentToolResult, TodoTool, TodoToolConfig};
    use crate::behavior::BehaviorLLMResult;
    use crate::step_record::LLMStepRecord;
    use crate::workspace::WorkshopWorkspaceRecord;
    use buckyos_api::{
        AccessGroupLevel, AccountBinding, AiMessage, AiPayload, Capability, CompleteRequest,
        Contact, ContactSource, ModelSpec, Requirements,
    };
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
            "__OPENDAN_VAR(session_id, $session_id)\n__OPENDAN_VAR(step_index, $step_index)\n__OPENDAN_VAR(current_todo, $current_todo)\nsid={{session_id}} step={{step_index}} todo={{current_todo}} x=__OPENDAN_ENV($params.x)__",
            &env_context,
            session,
        )
        .await
        .expect("render prompt");

        assert_eq!(result.rendered.trim(), "sid=s1 step=3 todo=T01 x=env_val");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.var_registered, 3);
        assert_eq!(result.resolved_vars.get("session_id"), Some(&true));
        assert_eq!(result.resolved_vars.get("step_index"), Some(&true));
        assert_eq!(result.resolved_vars.get("current_todo"), Some(&true));
    }

    #[tokio::test]
    async fn render_prompt_auto_registers_plain_placeholders() {
        let session = Arc::new(Mutex::new(AgentSession::new(
            "s-auto",
            "did:test:agent",
            Some("on_wakeup"),
        )));
        session.lock().await.step_index = 2;

        let result = AgentEnvironment::render_prompt(
            "sid={{session_id}} step={{step_index}} last={{last_step}}",
            &HashMap::new(),
            session,
        )
        .await
        .expect("render prompt");

        assert_eq!(result.rendered, "sid=s-auto step=2 last=");
        assert_eq!(result.resolved_vars.get("session_id"), Some(&true));
        assert_eq!(result.resolved_vars.get("step_index"), Some(&true));
        assert_eq!(result.resolved_vars.get("last_step"), Some(&false));
    }

    #[tokio::test]
    async fn render_text_template_requires_explicit_var_registration() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("session_id".to_string(), Json::String("s-demo".to_string()));

        let err = AgentEnvironment::render_text_template(
            "sid={{session_id}}",
            |_key| async { Ok(Some("from-loader".to_string())) },
            &env_context,
        )
        .await
        .expect_err("unregistered upon variable should fail");

        assert!(
            err.to_string().contains("not found in this scope"),
            "err={err}"
        );
    }

    #[tokio::test]
    async fn render_text_template_registers_context_for_upon_render() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params".to_string(), json!({ "message": "hello" }));

        let result = AgentEnvironment::render_text_template(
            "__OPENDAN_VAR(new_msg, $params.message)\n{%- if new_msg %}msg={{new_msg}}{%- endif %}",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered.trim(), "msg=hello");
        assert_eq!(result.env_expanded, 0);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.var_registered, 1);
        assert_eq!(result.var_failed, 0);
        assert_eq!(result.resolved_vars.get("new_msg"), Some(&true));
    }

    #[tokio::test]
    async fn render_text_template_supports_json_value_loaded_from_session() {
        let result = AgentEnvironment::render_text_template(
            "__OPENDAN_VAR(owner, $owner)\nname={{owner.show_name}} tg={{owner.contact.bindings.0.display_id}}",
            |key| {
                let key = key.to_string();
                async move {
                    if key == "owner" {
                        Ok(Some(
                            r#"{"user_id":"devtest","show_name":"Liu Zhicong","contact":{"bindings":[{"display_id":"wacer2026"}]}}"#
                                .to_string(),
                        ))
                    } else {
                        Ok(None)
                    }
                }
            },
            &HashMap::new(),
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered.trim(), "name=Liu Zhicong tg=wacer2026");
        assert_eq!(result.resolved_vars.get("owner"), Some(&true));
    }

    #[tokio::test]
    async fn load_value_from_session_step_record_renders_log_text() {
        let root = tempdir().expect("create temp dir");
        let session = Arc::new(Mutex::new(AgentSession::new(
            "work-step-record",
            "did:test:agent",
            Some("plan"),
        )));
        {
            let mut guard = session.lock().await;
            guard.session_root_dir = root.path().to_path_buf();
            guard
                .append_llm_step_record(LLMStepRecord {
                    session_id: "work-step-record".to_string(),
                    step_num: 0,
                    step_index: 0,
                    behavior_name: "plan".to_string(),
                    started_at_ms: 1,
                    llm_completed_at_ms: 2,
                    action_completed_at_ms: 3,
                    new_msg: vec![],
                    input: "task".to_string(),
                    llm_prompt: CompleteRequest::new(
                        Capability::LlmRouter,
                        ModelSpec::new("llm.default".to_string(), None),
                        Requirements::new(vec![], None, None, None),
                        AiPayload::new(
                            None,
                            vec![AiMessage::new("user".to_string(), "prompt-0".to_string())],
                            vec![],
                            vec![],
                            None,
                            None,
                        ),
                        Some("step-0".to_string()),
                    ),
                    llm_result: BehaviorLLMResult {
                        conclusion: Some("discover repo layout".to_string()),
                        thinking: Some("inspect source tree".to_string()),
                        shell_commands: vec!["rg -n AgentSession".to_string()],
                        ..Default::default()
                    },
                    action_result: HashMap::from([(
                        "#0".to_string(),
                        AgentToolResult::from_details(json!({}))
                            .with_cmd_line("rg -n AgentSession")
                            .with_result("rg output"),
                    )]),
                    error: None,
                })
                .await
                .expect("append step 0");
            guard
                .append_llm_step_record(LLMStepRecord {
                    session_id: "work-step-record".to_string(),
                    step_num: 1,
                    step_index: 1,
                    behavior_name: "plan".to_string(),
                    started_at_ms: 4,
                    llm_completed_at_ms: 5,
                    action_completed_at_ms: 6,
                    new_msg: vec![],
                    input: "continue".to_string(),
                    llm_prompt: CompleteRequest::new(
                        Capability::LlmRouter,
                        ModelSpec::new("llm.default".to_string(), None),
                        Requirements::new(vec![], None, None, None),
                        AiPayload::new(
                            None,
                            vec![AiMessage::new("user".to_string(), "prompt-1".to_string())],
                            vec![],
                            vec![],
                            None,
                            None,
                        ),
                        Some("step-1".to_string()),
                    ),
                    llm_result: BehaviorLLMResult {
                        conclusion: Some("AgentSession needs a step log field".to_string()),
                        thinking: Some("wire session storage and template access".to_string()),
                        shell_commands: vec!["patch_files".to_string()],
                        ..Default::default()
                    },
                    action_result: HashMap::from([(
                        "#1".to_string(),
                        AgentToolResult::from_details(json!({}))
                            .with_cmd_line("patch_files")
                            .with_result("patched files"),
                    )]),
                    error: None,
                })
                .await
                .expect("append step 1");
        }

        let rendered = AgentEnvironment::load_value_from_session(session.clone(), "step_record")
            .await
            .expect("load step_record")
            .expect("step_record should exist");
        assert!(rendered.contains("<steps_summary>"), "rendered={rendered}");
        assert!(
            rendered.contains("AgentSession needs a step log field"),
            "rendered={rendered}"
        );

        let last = AgentEnvironment::load_value_from_session(session.clone(), "step_record.last")
            .await
            .expect("load step_record.last")
            .expect("step_record.last should exist");
        assert!(last.contains("patch_files"), "last={last}");
        assert!(last.contains("success"), "last={last}");
        assert!(last.contains("wire session storage"), "last={last}");

        let alias_last = AgentEnvironment::load_value_from_session(session.clone(), "last_step")
            .await
            .expect("load last_step")
            .expect("last_step should exist");
        assert!(
            alias_last.contains("patch_files"),
            "alias_last={alias_last}"
        );

        let alias_steps = AgentEnvironment::load_value_from_session(session, "last_steps.$2")
            .await
            .expect("load last_steps.$2")
            .expect("last_steps.$2 should exist");
        assert!(
            alias_steps.contains("<steps_summary>"),
            "alias_steps={alias_steps}"
        );
        assert!(
            alias_steps.contains("discover repo layout"),
            "alias_steps={alias_steps}"
        );
    }

    #[tokio::test]
    async fn render_text_template_loads_content_and_reprocesses_nested_directives() {
        let root = tempdir().expect("create temp dir");
        let include_path = root.path().join("prompt.md");
        fs::write(&include_path, "Project __OPENDAN_ENV($params.project)__")
            .await
            .expect("write include file");

        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert(
            "params.project".to_string(),
            Json::String("Alpha".to_string()),
        );
        env_context.insert(
            "dynamic.path".to_string(),
            Json::String(include_path.to_string_lossy().to_string()),
        );

        let result = AgentEnvironment::render_text_template(
            "__OPENDAN_CONTENT($dynamic.path)__",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "Project Alpha");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 0);
        assert_eq!(result.content_loaded, 1);
        assert_eq!(result.var_registered, 0);
    }

    #[tokio::test]
    async fn render_text_template_missing_content_file_counts_as_failed_and_renders_empty() {
        let root = tempdir().expect("create temp dir");
        let missing_path = root.path().join("missing.md");

        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert(
            "dynamic.path".to_string(),
            Json::String(missing_path.to_string_lossy().to_string()),
        );

        let result = AgentEnvironment::render_text_template(
            "before __OPENDAN_CONTENT($dynamic.path)__ after",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "before  after");
        assert_eq!(result.content_loaded, 0);
        assert_eq!(result.content_failed, 1);
    }

    #[tokio::test]
    async fn render_text_template_executes_command_and_expands_dynamic_paths() {
        let root = tempdir().expect("create temp dir");
        let memory_dir = root.path().join("memory");
        let result_path = memory_dir.join("result.txt");
        fs::create_dir_all(&memory_dir)
            .await
            .expect("create memory dir");
        fs::write(&result_path, "command-result")
            .await
            .expect("write command result");

        let agent_root = root.path().to_path_buf();
        let result = AgentEnvironment::render_text_template(
            r#"__OPENDAN_EXEC("cat $agent_root/memory/result.txt")__"#,
            move |key| {
                let agent_root = agent_root.clone();
                let key = key.to_string();
                async move {
                    if let Some(rel_path) = key.strip_prefix("$agent_root/") {
                        return Ok(Some(
                            agent_root.join(rel_path).to_string_lossy().to_string(),
                        ));
                    }
                    Ok(None)
                }
            },
            &HashMap::new(),
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "command-result");
    }

    #[tokio::test]
    async fn render_text_template_exec_failure_returns_error() {
        let err = AgentEnvironment::render_text_template(
            r#"__OPENDAN_EXEC("printf fail >&2; exit 7")__"#,
            |_key| async { Ok(None) },
            &HashMap::new(),
        )
        .await
        .expect_err("exec failure should abort render");

        assert!(
            err.to_string()
                .contains("__OPENDAN_EXEC command failed: exit_code=7"),
            "err={err}"
        );
    }

    #[tokio::test]
    async fn render_text_template_env_not_found_counts_separately() {
        let mut env_context = HashMap::<String, Json>::new();
        env_context.insert("params.todo".to_string(), Json::String("T01".to_string()));

        let result = AgentEnvironment::render_text_template(
            "a=__OPENDAN_ENV($params.todo)__ b=__OPENDAN_ENV($params.missing)__",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered, "a=T01 b=");
        assert_eq!(result.env_expanded, 1);
        assert_eq!(result.env_not_found, 1);
        assert_eq!(result.content_loaded, 0);
        assert_eq!(result.var_registered, 0);
    }

    #[test]
    fn load_value_from_session_new_msg_respects_numeric_pull_limit() {
        assert_eq!(
            parse_pull_limit_from_key("new_msg.2", "new_msg", DEFAULT_NEW_MSG_MAX_PULL),
            2
        );
        assert_eq!(
            parse_pull_limit_from_key("new_msg.0", "new_msg", DEFAULT_NEW_MSG_MAX_PULL),
            DEFAULT_NEW_MSG_MAX_PULL
        );
    }

    #[test]
    fn load_value_from_session_new_msg_supports_dollar_pull_limit() {
        assert_eq!(
            parse_pull_limit_from_key("new_msg.$3", "new_msg", DEFAULT_NEW_MSG_MAX_PULL),
            3
        );
        assert_eq!(
            parse_pull_limit_from_key("new_msg.$99999", "new_msg", DEFAULT_NEW_MSG_MAX_PULL),
            4096
        );
    }

    #[test]
    fn load_value_from_session_new_event_respects_numeric_pull_limit() {
        assert_eq!(
            parse_pull_limit_from_key("new_event.2", "new_event", DEFAULT_NEW_EVENT_MAX_PULL),
            2
        );
        assert_eq!(
            parse_pull_limit_from_key("new_event.0", "new_event", DEFAULT_NEW_EVENT_MAX_PULL),
            DEFAULT_NEW_EVENT_MAX_PULL
        );
    }

    #[test]
    fn load_value_from_session_new_event_supports_dollar_pull_limit() {
        assert_eq!(
            parse_pull_limit_from_key("new_event.$3", "new_event", DEFAULT_NEW_EVENT_MAX_PULL),
            3
        );
        assert_eq!(
            parse_pull_limit_from_key("new_event.$99999", "new_event", DEFAULT_NEW_EVENT_MAX_PULL),
            4096
        );
    }

    #[test]
    fn render_new_msg_prefers_from_name_as_sender() {
        let record: MsgRecord = serde_json::from_value(json!({
            "record_id": "rid-1",
            "box_kind": "INBOX",
            "msg_id": "msg:01",
            "state": "UNREAD",
            "from": "did:bns:alice",
            "from_name": "Alice",
            "to": "did:bns:jarvis",
            "created_at_ms": 1_735_689_600_000u64,
            "updated_at_ms": 1_735_689_600_000u64,
            "sort_key": 1_735_689_600_000u64
        }))
        .expect("parse msg record");

        let rendered = render_human_readable_msg_line(&record, None);
        assert!(rendered.starts_with("Alice ["), "rendered={}", rendered);
        assert!(!rendered.starts_with("alice ["), "rendered={}", rendered);
    }

    #[test]
    fn render_new_event_formats_id_and_payload() {
        let payload = serde_json::to_vec(&SessionInputItem {
            msg: None,
            event_id: Some("/taskmgr/new/task_001".to_string()),
            event_data: Some(json!({
                "task_id": "task_001",
                "status": "created"
            })),
        })
        .expect("serialize session input");
        let rendered = render_new_events_from_kmsgqueue(&[Message {
            index: 1,
            payload,
            created_at: 1_735_689_600_000u64,
            headers: HashMap::new(),
        }])
        .expect("rendered event");

        let parsed: Json = serde_json::from_str(&rendered).expect("parse rendered event json");
        assert_eq!(
            parsed["eventid"],
            Json::String("/taskmgr/new/task_001".to_string())
        );
        assert_eq!(
            parsed["eventdata"]["task_id"],
            Json::String("task_001".to_string())
        );
        assert_eq!(
            parsed["eventdata"]["status"],
            Json::String("created".to_string())
        );
    }

    #[test]
    fn render_new_event_omits_missing_eventdata() {
        let payload = serde_json::to_vec(&SessionInputItem {
            msg: None,
            event_id: Some("/taskmgr/ping".to_string()),
            event_data: None,
        })
        .expect("serialize session input");
        let rendered = render_new_events_from_kmsgqueue(&[Message {
            index: 1,
            payload,
            created_at: 1_735_689_600_000u64,
            headers: HashMap::new(),
        }])
        .expect("rendered event");

        let parsed: Json = serde_json::from_str(&rendered).expect("parse rendered event json");
        assert_eq!(parsed["eventid"], Json::String("/taskmgr/ping".to_string()));
        assert!(parsed.get("eventdata").is_none(), "rendered={rendered}");
    }

    #[tokio::test]
    async fn load_value_from_session_workspace_todolist_todo_ref_reads_db_with_session_scope() {
        let root = tempdir().expect("create temp dir");
        let agent_env_root = root.path();
        let local_workspace_id = "ws-demo";
        let session_id = "s-work";
        let todo_db_path = agent_env_root.join("todo").join("todo.db");
        let workspace_dir = agent_env_root.join("workspaces").join(local_workspace_id);
        std::fs::create_dir_all(&workspace_dir).expect("create local workspace path");

        // Ensure todo schema exists.
        let _tool = TodoTool::new(TodoToolConfig::with_db_path(todo_db_path.clone()))
            .expect("init todo tool");
        let conn = Connection::open(&todo_db_path).expect("open todo db");

        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "todo-1",
                local_workspace_id,
                session_id,
                "T001",
                "Implement parser",
                "Task",
                "WAIT",
                "did:test:agent",
                1000_i64,
                1000_i64,
                "root_agent",
                "did:test:agent"
            ],
        )
        .expect("insert todo");

        let mut s = AgentSession::new(session_id, "did:test:agent", Some("on_wakeup"));
        s.local_workspace_id = Some(local_workspace_id.to_string());
        s.pwd = workspace_dir;
        s.workspace_info = Some(json!({
            "binding": {
                "local_workspace_id": local_workspace_id,
                "workspace_path": agent_env_root.join("workspaces").join(local_workspace_id),
                "workspace_rel_path": format!("workspaces/{local_workspace_id}"),
                "agent_env_root": agent_env_root
            }
        }));
        let session = Arc::new(Mutex::new(s));

        let rendered =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace.todolist.T001")
                .await
                .expect("load todo by ref")
                .expect("todo text should exist");
        assert!(
            rendered.contains("Current Todo T001 [WAIT]"),
            "rendered={rendered}"
        );

        let mut s_other = AgentSession::new("s-other", "did:test:agent", Some("on_wakeup"));
        s_other.local_workspace_id = Some(local_workspace_id.to_string());
        s_other.pwd = agent_env_root.join("workspaces").join(local_workspace_id);
        s_other.workspace_info = Some(json!({
            "binding": {
                "local_workspace_id": local_workspace_id,
                "workspace_path": agent_env_root.join("workspaces").join(local_workspace_id),
                "workspace_rel_path": format!("workspaces/{local_workspace_id}"),
                "agent_env_root": agent_env_root
            }
        }));
        let session_other = Arc::new(Mutex::new(s_other));
        let hidden =
            AgentEnvironment::load_value_from_session(session_other, "workspace.todolist.T001")
                .await
                .expect("query other session todo");
        assert!(hidden.is_none(), "todo from other session should be hidden");

        let list_rendered =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace_todolist")
                .await
                .expect("load workspace_todolist")
                .expect("workspace_todolist should exist");
        assert!(
            list_rendered.starts_with("Workspace Todo (ws-demo, v0)"),
            "list_rendered={list_rendered}"
        );
        assert!(
            list_rendered.contains("T001 [WAIT]"),
            "list_rendered={list_rendered}"
        );

        let todo_code =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace_current_todo_id")
                .await
                .expect("load workspace_current_todo_id")
                .expect("workspace_current_todo_id should exist");
        assert_eq!(todo_code, "T001");

        let next_ready =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace_next_ready_todo")
                .await
                .expect("load workspace_next_ready_todo")
                .expect("workspace_next_ready_todo should exist");
        assert!(
            next_ready.contains("Current Todo T001 [WAIT]"),
            "next_ready={next_ready}"
        );

        let alias_rendered =
            AgentEnvironment::load_value_from_session(session, "workspace_todolist.T001")
                .await
                .expect("load workspace_todolist.T001")
                .expect("workspace_todolist.T001 should exist");
        assert!(
            alias_rendered.contains("Current Todo T001 [WAIT]"),
            "alias_rendered={alias_rendered}"
        );
    }

    #[tokio::test]
    async fn render_text_template_var_missing_counts_as_failed() {
        let env_context = HashMap::<String, Json>::new();

        let result = AgentEnvironment::render_text_template(
            "__OPENDAN_VAR(todo, $workspace.todolist.T001)\ntodo={{todo}}",
            |_key| async { Ok(None) },
            &env_context,
        )
        .await
        .expect("render text template");

        assert_eq!(result.rendered.trim(), "todo=");
        assert_eq!(result.var_registered, 0);
        assert_eq!(result.var_failed, 1);
        assert_eq!(result.resolved_vars.get("todo"), Some(&false));
    }

    #[tokio::test]
    async fn render_text_template_rejects_non_dollar_env_arg() {
        let err = AgentEnvironment::render_text_template(
            "__OPENDAN_ENV(params.todo)__",
            |_key| async { Ok(None) },
            &HashMap::new(),
        )
        .await
        .expect_err("env arg without `$` should fail");

        assert!(
            err.to_string()
                .contains("__OPENDAN_ENV expects a dynamic variable expression"),
            "err={err}"
        );
    }

    #[tokio::test]
    async fn render_text_template_rejects_relative_content_path() {
        let err = AgentEnvironment::render_text_template(
            "__OPENDAN_CONTENT(./prompt.md)__",
            |_key| async { Ok(None) },
            &HashMap::new(),
        )
        .await
        .expect_err("relative content path should fail");

        assert!(
            err.to_string()
                .contains("__OPENDAN_CONTENT expects a dynamic variable or absolute path"),
            "err={err}"
        );
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
    async fn render_text_replaces_new_event_variable() {
        let root = tempdir().expect("create temp dir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create env");
        let ctx = PromptTemplateContext {
            new_event: Some("/taskmgr/new/task_001".to_string()),
            ..PromptTemplateContext::default()
        };

        let rendered = env
            .render_prompt_template("Event={{new_event}}", TemplateRenderMode::Text, &ctx)
            .await
            .expect("render template")
            .expect("text mode should return string");

        assert_eq!(rendered, "Event=/taskmgr/new/task_001");
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
        let session_root = root.path().join("sessions");
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

        write_session_record(&session_root, "work-s1", "Session 1", "Summary 1", 100).await;
        write_session_record(&session_root, "work-s2", "Session 2", "Summary 2", 300).await;
        write_session_record(&session_root, "work-s3", "Session 3", "Summary 3", 200).await;
        write_session_record(&session_root, "ui-chat-1", "UI Session", "UI Summary", 500).await;
        write_session_record(
            &session_root,
            "tg:lzc_jarvis:5397330802",
            "TG Session",
            "",
            600,
        )
        .await;

        let session = Arc::new(Mutex::new(AgentSession::new(
            "work-s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));
        session.lock().await.pwd = root.path().to_path_buf();

        let rendered = AgentEnvironment::load_value_from_session(session.clone(), "session_list")
            .await
            .expect("load session_list")
            .expect("session_list should be rendered");

        let lines = rendered.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("- work-s2 : Session 2  "));
        assert!(lines[1].starts_with("- work-s3 : Session 3  "));
        assert!(lines[2].starts_with("- work-s1 : Session 1  "));
        assert!(!rendered.contains("ui-chat-1"));
        assert!(!rendered.contains("tg:lzc_jarvis:5397330802"));

        let rendered_2 = AgentEnvironment::load_value_from_session(session, "session_list.$2")
            .await
            .expect("load session_list.$2")
            .expect("session_list.$2 should be rendered");
        let lines_2 = rendered_2.lines().collect::<Vec<_>>();
        assert_eq!(lines_2.len(), 2);
        assert!(lines_2[0].starts_with("- work-s2 : Session 2  "));
        assert!(lines_2[1].starts_with("- work-s3 : Session 3  "));
    }

    #[tokio::test]
    async fn load_value_from_session_local_workspace_list_sorts_recent_first() {
        let root = tempdir().expect("create temp dir");
        fs::create_dir_all(root.path().join("workspaces"))
            .await
            .expect("create local workspace dir");

        let mut ws1 = WorkshopWorkspaceRecord::default();
        ws1.workspace_id = "local-alpha-1".to_string();
        ws1.workspace_type = WorkspaceType::Local;
        ws1.name = "Alpha workspace".to_string();
        ws1.created_at_ms = 10;
        ws1.updated_at_ms = 100;

        let mut ws2 = WorkshopWorkspaceRecord::default();
        ws2.workspace_id = "local-beta-1".to_string();
        ws2.workspace_type = WorkspaceType::Local;
        ws2.name = "Beta workspace".to_string();
        ws2.created_at_ms = 20;
        ws2.updated_at_ms = 300;

        let mut ws3 = WorkshopWorkspaceRecord::default();
        ws3.workspace_id = "local-gamma-1".to_string();
        ws3.workspace_type = WorkspaceType::Local;
        ws3.name = "Gamma workspace".to_string();
        ws3.created_at_ms = 30;
        ws3.updated_at_ms = 200;

        let mut remote = WorkshopWorkspaceRecord::default();
        remote.workspace_id = "remote-1".to_string();
        remote.workspace_type = WorkspaceType::Remote;
        remote.name = "Remote workspace".to_string();
        remote.created_at_ms = 40;
        remote.updated_at_ms = 999;

        let index = WorkshopIndex {
            agent_did: "did:test:agent".to_string(),
            workspaces: vec![ws1, ws2, ws3, remote],
            updated_at_ms: 999,
        };
        let index_bytes = serde_json::to_vec_pretty(&index).expect("serialize workshop index");
        fs::write(root.path().join(WORKSHOP_INDEX_FILE_NAME), index_bytes)
            .await
            .expect("write workshop index");

        let session = Arc::new(Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("on_wakeup"),
        )));
        session.lock().await.pwd = root.path().to_path_buf();

        let rendered =
            AgentEnvironment::load_value_from_session(session.clone(), "local_workspace_list")
                .await
                .expect("load local_workspace_list")
                .expect("local_workspace_list should be rendered");
        let beta_idx = rendered.find("$local-beta-1").expect("beta workspace");
        let gamma_idx = rendered.find("$local-gamma-1").expect("gamma workspace");
        let alpha_idx = rendered.find("$local-alpha-1").expect("alpha workspace");
        assert!(beta_idx < gamma_idx, "rendered={}", rendered);
        assert!(gamma_idx < alpha_idx, "rendered={}", rendered);
        assert!(rendered.contains("Beta workspace"), "rendered={}", rendered);
        assert!(
            rendered.contains("Gamma workspace"),
            "rendered={}",
            rendered
        );
        assert!(
            rendered.contains("Alpha workspace"),
            "rendered={}",
            rendered
        );

        let rendered_2 =
            AgentEnvironment::load_value_from_session(session.clone(), "local_workspace_list.$2")
                .await
                .expect("load local_workspace_list.$2")
                .expect("local_workspace_list.$2 should be rendered");
        assert!(
            rendered_2.contains("$local-beta-1"),
            "rendered={}",
            rendered_2
        );
        assert!(
            rendered_2.contains("$local-gamma-1"),
            "rendered={}",
            rendered_2
        );
        assert!(
            !rendered_2.contains("$local-alpha-1"),
            "rendered={}",
            rendered_2
        );

        let rendered_alias =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace_list.$2")
                .await
                .expect("load workspace_list.$2")
                .expect("workspace_list.$2 should be rendered");
        assert_eq!(rendered_alias, rendered_2);
    }

    #[tokio::test]
    async fn load_value_from_session_supports_session_aliases_and_session_root_path() {
        let root = tempdir().expect("create temp dir");
        let sessions_root = root.path().join("sessions");
        fs::create_dir_all(sessions_root.join("sess-alias"))
            .await
            .expect("create session dir");

        let session = Arc::new(Mutex::new(AgentSession::new(
            "sess-alias",
            "did:test:agent",
            Some("plan"),
        )));
        {
            let mut guard = session.lock().await;
            guard.title = "Alias Session".to_string();
            guard.step_num = 7;
            guard.current_behavior = "check".to_string();
            guard.pwd = root.path().to_path_buf();
            guard.session_root_dir = sessions_root.clone();
        }

        let title = AgentEnvironment::load_value_from_session(session.clone(), "session_title")
            .await
            .expect("load session_title")
            .expect("session_title should exist");
        assert_eq!(title, "Alias Session");

        let behavior =
            AgentEnvironment::load_value_from_session(session.clone(), "current_behavior")
                .await
                .expect("load current_behavior")
                .expect("current_behavior should exist");
        assert_eq!(behavior, "check");

        let step_num = AgentEnvironment::load_value_from_session(session.clone(), "step_num")
            .await
            .expect("load step_num")
            .expect("step_num should exist");
        assert_eq!(step_num, "7");

        let session_summary_path =
            AgentEnvironment::load_value_from_session(session, "$session_root/summary.md")
                .await
                .expect("load $session_root/summary.md")
                .expect("$session_root/summary.md should resolve");
        assert_eq!(
            session_summary_path,
            sessions_root
                .join("sess-alias")
                .join("summary.md")
                .to_string_lossy()
        );
    }

    #[tokio::test]
    async fn load_value_from_session_supports_workspace_root_alias_and_path() {
        let root = tempdir().expect("create temp dir");
        let workspace_root = root.path().join("workspaces").join("ws-alias");

        let session = Arc::new(Mutex::new(AgentSession::new(
            "sess-workspace-root",
            "did:test:agent",
            Some("plan"),
        )));
        {
            let mut guard = session.lock().await;
            guard.pwd = root.path().to_path_buf();
            guard.workspace_info = Some(json!({
                "binding": {
                    "local_workspace_id": "ws-alias",
                    "workspace_rel_path": "workspaces/ws-alias",
                    "agent_env_root": root.path(),
                }
            }));
        }

        let rendered_root =
            AgentEnvironment::load_value_from_session(session.clone(), "workspace_root")
                .await
                .expect("load workspace_root")
                .expect("workspace_root should resolve");
        assert_eq!(rendered_root, workspace_root.to_string_lossy());

        let rendered_child =
            AgentEnvironment::load_value_from_session(session, "$workspace_root/objective")
                .await
                .expect("load $workspace_root/objective")
                .expect("$workspace_root/objective should resolve");
        assert_eq!(
            rendered_child,
            workspace_root.join("objective").to_string_lossy()
        );
    }

    #[test]
    fn build_owner_json_value_exposes_show_name_and_bindings() {
        let contact = Contact {
            did: DID::new("bns", "alice"),
            name: "Alice".to_string(),
            avatar: Some("https://example.com/avatar.png".to_string()),
            note: Some("Primary owner".to_string()),
            source: ContactSource::ManualCreate,
            is_verified: true,
            bindings: vec![
                AccountBinding {
                    platform: "telegram".to_string(),
                    account_id: "alice_001".to_string(),
                    display_id: "@alice".to_string(),
                    tunnel_id: "did:bns:tg-tunnel".to_string(),
                    last_active_at: 0,
                    meta: HashMap::new(),
                },
                AccountBinding {
                    platform: "email".to_string(),
                    account_id: "alice@example.com".to_string(),
                    display_id: "alice@example.com".to_string(),
                    tunnel_id: "did:bns:email-tunnel".to_string(),
                    last_active_at: 0,
                    meta: HashMap::new(),
                },
            ],
            access_level: AccessGroupLevel::Friend,
            temp_grants: vec![],
            groups: vec![],
            tags: vec!["owner".to_string(), "trusted".to_string()],
            created_at: 1,
            updated_at: 2,
        };

        let value = build_owner_json_value(&contact.did, Some("devtest"), Some(&contact));
        assert_eq!(value["user_id"], json!("devtest"));
        assert_eq!(value["did"], json!("did:bns:alice"));
        assert_eq!(value["show_name"], json!("Alice"));
        assert_eq!(value["contact"]["did"], json!("did:bns:alice"));
        assert_eq!(value["contact"]["note"], json!("Primary owner"));
        assert_eq!(
            value["contact"]["bindings"][0]["display_id"],
            json!("@alice")
        );
        assert_eq!(
            value["contact"]["bindings"][1]["account_id"],
            json!("alice@example.com")
        );
    }
}
