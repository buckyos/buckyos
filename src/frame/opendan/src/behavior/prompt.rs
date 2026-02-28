use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{
    features, AiMessage, AiPayload, AiToolSpec, BoxKind, Capability, CompleteRequest, ModelSpec,
    MsgRecordWithObject, Requirements,
};
use chrono::Utc;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::fs;
use tokio::sync::Mutex;
use tokio::task;

use crate::agent_environment::AgentEnvironment;
use crate::agent_memory::AgentMemory;
use crate::agent_session::AgentSession;
use crate::agent_tool::{normalize_tool_name, ActionSpec};
use crate::behavior::config::BehaviorMemoryBucketConfig;
use crate::behavior::BehaviorConfig;
use crate::worklog::{WorklogListOptions, WorklogRecord, WorklogTool, WorklogToolConfig};
use crate::workspace::todo::render_workspace_todo_prompt_from_db;

use super::sanitize::{sanitize_json_compact, sanitize_text};
use super::types::{BehaviorExecInput, LLMBehaviorConfig};
use super::Tokenizer;

const SESSION_MSG_RECORD_FILES: [&str; 2] = ["msg_record.jsonl", "message_record.jsonl"];
const SKILL_SPEC_EXTENSIONS: [&str; 3] = ["yaml", "yml", "json"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub name: Option<String>,
    pub content: String,
}

pub struct PromptBuilder;

impl PromptBuilder {
    //通过理解下面实现，可以理解OpenDAN Agent的整体设计
    pub async fn build(
        input: &BehaviorExecInput,
        tools: &[AiToolSpec],
        action_specs: &[ActionSpec],
        cfg: &BehaviorConfig,
        tokenizer: &dyn Tokenizer,
        session: Option<Arc<Mutex<AgentSession>>>,
        memory: Option<AgentMemory>,
    ) -> Result<CompleteRequest, String> {
        let env_context = build_env_context(input);
        let mut loaded_tools = Vec::new();

        let process_rules =
            render_section(cfg.process_rule.as_str(), &env_context, session.clone()).await?;

        let role_text = render_section(
            format!("{}\n\n{}", input.role_md, input.self_md).as_str(),
            &env_context,
            session.clone(),
        )
        .await?;

        let policy_text = if cfg.policy.trim().is_empty() {
            String::new()
        } else {
            render_section(cfg.policy.as_str(), &env_context, session.clone()).await?
        };

        let output_protocol_text = cfg.output_protocol.to_prompt_text();

        let mut system_parts = vec![
            format!("<<role>>\n{}\n<</role>>", sanitize_text(role_text.as_str())),
            format!(
                "<<process_rules>>\n{}\n<</process_rules>>",
                sanitize_text(process_rules.as_str())
            ),
        ];
        if !policy_text.is_empty() {
            system_parts.push(format!(
                "<<policy>>\n{}\n<</policy>>",
                sanitize_text(policy_text.as_str())
            ));
        }
        system_parts.push(format!(
            "<<output_protocol>>\n{}\n<</output_protocol>>",
            sanitize_text(output_protocol_text.as_str())
        ));

        if let Some((toolbox, tools)) =
            build_toolbox(tools, action_specs, cfg, session.clone()).await
        {
            system_parts.push(format!("<<toolbox>>\n{}\n<</toolbox>>", toolbox));
            loaded_tools = tools;
        }

        let system_role_prompt_text = system_parts.join("\n\n");
        let tool_define_used = 1024;
        let system_used_token = tokenizer.count_tokens(system_role_prompt_text.as_str());
        let input_used_token = tokenizer.count_tokens(&input.input_prompt);
        let memory_token_limit = cfg
            .limits
            .max_prompt_tokens
            .saturating_sub(system_used_token)
            .saturating_sub(input_used_token)
            .saturating_sub(tool_define_used);

        let memory_prompt_text =
            build_memory_prompt_text(input, cfg, vec![], memory_token_limit, tokenizer, memory)
                .await;

        let ai_messages: Vec<AiMessage> = vec![
            AiMessage::new("system".to_string(), system_role_prompt_text),
            AiMessage::new("user".to_string(), memory_prompt_text),
            AiMessage::new(
                "user".to_string(),
                format!("<<input>>\n{}\n<</input>>", input.input_prompt),
            ),
        ];

        let mut must_features = vec![features::JSON_OUTPUT.to_string()];
        if !loaded_tools.is_empty() {
            must_features.push(features::TOOL_CALLING.to_string());
        }

        let mut options = Map::new();
        options.insert(
            "max_completion_tokens".to_string(),
            json!(input.limits.max_completion_tokens),
        );
        options.insert(
            "temperature".to_string(),
            json!(cfg.llm.model_policy.temperature),
        );

        let req = CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new(cfg.llm.model_policy.preferred.clone(), None),
            Requirements::new(must_features, None, None, None),
            AiPayload::new(
                None,
                ai_messages,
                loaded_tools,
                vec![],
                None,
                Some(Json::Object(options)),
            ),
            Some(format!(
                "{}:{}:{}:{}",
                input.trace.trace_id,
                input.trace.wakeup_id,
                input.trace.behavior,
                input.trace.step_idx
            )),
        );
        Ok(req)
    }
}

fn build_env_context(input: &BehaviorExecInput) -> HashMap<String, Json> {
    let mut ctx = HashMap::new();
    ctx.insert("role_md".to_string(), Json::String(input.role_md.clone()));
    ctx.insert("self_md".to_string(), Json::String(input.self_md.clone()));
    if let Some(ref sid) = input.session_id {
        ctx.insert("session_id".to_string(), Json::String(sid.clone()));
        ctx.insert("loop.session_id".to_string(), Json::String(sid.clone()));
    }
    ctx.insert(
        "step.index".to_string(),
        Json::String(input.trace.step_idx.to_string()),
    );
    ctx
}

async fn render_section(
    template: &str,
    env_context: &HashMap<String, Json>,
    session: Option<Arc<Mutex<AgentSession>>>,
) -> Result<String, String> {
    if template.trim().is_empty() {
        return Ok(String::new());
    }
    let Some(session) = session else {
        return Ok(template.to_string());
    };
    let result = AgentEnvironment::render_prompt(template, env_context, session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rendered)
}

/// Build memory prompt with dynamic compression skeleton.
async fn build_memory_prompt_text(
    input: &BehaviorExecInput,
    cfg: &BehaviorConfig,
    topic_tags: Vec<String>,
    memory_limit: u32,
    tokenizer: &dyn Tokenizer,
    memory: Option<AgentMemory>,
) -> String {
    let total_budget = memory_limit.min(cfg.memory.total_limit);
    if total_budget == 0 {
        return String::new();
    }

    let workspace_summary_budget =
        calc_memory_bucket_budget(total_budget, &cfg.memory.workspace_summary);
    let agent_memory_budget = calc_memory_bucket_budget(total_budget, &cfg.memory.agent_memory);
    let history_messages_budget =
        calc_memory_bucket_budget(total_budget, &cfg.memory.history_messages);
    let workspace_worklog_budget =
        calc_memory_bucket_budget(total_budget, &cfg.memory.workspace_worklog);
    let session_summaries_budget =
        calc_memory_bucket_budget(total_budget, &cfg.memory.session_summaries);
    let workspace_todo_budget = calc_memory_bucket_budget(total_budget, &cfg.memory.workspace_todo);

    let mut memory_sections = Vec::<String>::new();

    if workspace_summary_budget > 0 {
        let workspace_summary =
            load_workspace_summary_with_limit(input, workspace_summary_budget, tokenizer).await;
        if !workspace_summary.trim().is_empty() {
            memory_sections.push(format!(
                "## Workspace Introduce\n{}\n",
                sanitize_text(workspace_summary.trim())
            ));
        }
    }

    let mut timeline_records = Vec::<MemoryTimelineRecord>::new();
    if agent_memory_budget > 0 {
        timeline_records.extend(
            load_agent_memory_with_limit(
                input,
                topic_tags.as_slice(),
                agent_memory_budget,
                tokenizer,
                memory.clone(),
            )
            .await,
        );
    }
    if history_messages_budget > 0 {
        timeline_records.extend(
            load_history_messages_with_limit(input, history_messages_budget, tokenizer).await,
        );
    }
    if workspace_worklog_budget > 0 {
        timeline_records.extend(
            load_workspace_worklog_with_limit(input, workspace_worklog_budget, tokenizer).await,
        );
    }
    timeline_records.sort_by(|a, b| {
        a.update_time
            .cmp(&b.update_time)
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    if !timeline_records.is_empty() {
        let timeline_text = timeline_records
            .iter()
            .map(format_memory_timeline_record)
            .collect::<Vec<_>>()
            .join("\n");
        memory_sections.push(format!(
            "## Timeline\n{}\n",
            sanitize_text(timeline_text.trim())
        ));
    }

    let mut session_summary_parts = Vec::<String>::new();
    if session_summaries_budget > 0 {
        let session_summary =
            load_session_summaries_with_limit(input, session_summaries_budget, tokenizer).await;
        if !session_summary.trim().is_empty() {
            session_summary_parts.push(format!(
                "## Session Summary\n{}\n",
                sanitize_text(session_summary.trim())
            ));
        }
    }

    if workspace_todo_budget > 0 {
        let workspace_todo =
            load_workspace_todo_with_limit(input, workspace_todo_budget, tokenizer).await;
        if !workspace_todo.trim().is_empty() {
            session_summary_parts.push(format!(
                "## TODO List\n{}\n",
                sanitize_text(workspace_todo.trim())
            ));
        }
    }
    if !session_summary_parts.is_empty() {
        memory_sections.push(session_summary_parts.join("\n"));
    }

    if memory_sections.is_empty() {
        return String::new();
    }

    let assembled = format!("<<memory>>\n{}\n<</memory>>", memory_sections.join("\n\n"));
    if tokenizer.count_tokens(assembled.as_str()) <= total_budget {
        assembled
    } else {
        truncate_to_token_budget(assembled.as_str(), total_budget)
    }
}

#[derive(Clone, Debug)]
struct MemoryTimelineRecord {
    update_time: u64,
    text: String,
    source_label: &'static str,
    source_order: u8,
}

fn is_memory_bucket_enabled(bucket: &BehaviorMemoryBucketConfig) -> bool {
    bucket.is_enable || bucket.limit > 0 || bucket.max_percent.is_some()
}

fn calc_memory_bucket_budget(total_budget: u32, bucket: &BehaviorMemoryBucketConfig) -> u32 {
    if total_budget == 0 || !is_memory_bucket_enabled(bucket) {
        return 0;
    }
    let by_percent = bucket
        .max_percent
        .map(|percent| ((total_budget as f64) * (percent as f64)) as u32)
        .unwrap_or(total_budget);
    let by_percent = by_percent.clamp(1, total_budget);
    if bucket.limit == 0 {
        by_percent
    } else {
        by_percent.min(bucket.limit).max(1)
    }
}

fn format_memory_timeline_record(record: &MemoryTimelineRecord) -> String {
    let line = record.text.trim();
    if line.is_empty() {
        return String::new();
    }
    format!("[{}][{}] {}", record.source_label, record.update_time, line)
}

async fn load_workspace_summary_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> String {
    if token_limit == 0 {
        return String::new();
    }
    let Some(session) = input.session.as_ref() else {
        return String::new();
    };

    let (local_workspace_id, workspace_info) = {
        let guard = session.lock().await;
        (
            guard.local_workspace_id.clone(),
            guard.workspace_info.clone(),
        )
    };

    let mut lines = Vec::<String>::new();
    if let Some(workspace_info) = workspace_info.as_ref() {
        if lines.is_empty() {
            lines.push(format!("{}", sanitize_json_compact(workspace_info)));
        }
    }

    fit_text_with_token_limit(lines.join("\n"), token_limit, tokenizer)
}

async fn load_agent_memory_with_limit(
    _input: &BehaviorExecInput,
    topic_tags: &[String],
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
    memory: Option<AgentMemory>,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }
    let Some(memory) = memory else {
        return vec![];
    };

    let tags = topic_tags
        .iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    let current_time = Utc::now();
    let items = match memory
        .load_memory(Some(token_limit), tags, Some(current_time))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            warn!("prompt.load_agent_memory load_memory failed: {}", err);
            return vec![];
        }
    };
    let raw_text = AgentMemory::render_memory_items(&items);
    let fitted = fit_text_with_token_limit(raw_text, token_limit, tokenizer);
    if fitted.trim().is_empty() {
        return vec![];
    }

    let update_time = current_time.timestamp_millis().max(0) as u64;
    fitted
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| MemoryTimelineRecord {
            update_time,
            text: line.to_string(),
            source_label: "memory",
            source_order: 10,
        })
        .collect()
}

async fn load_session_summaries_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> String {
    if token_limit == 0 {
        return String::new();
    }
    let Some(session) = input.session.as_ref() else {
        return String::new();
    };

    let (session_id, title, summary, last_step_summary, state, current_behavior, step_index, links) = {
        let guard = session.lock().await;
        (
            guard.session_id.clone(),
            guard.title.clone(),
            guard.summary.clone(),
            guard.last_step_summary.clone(),
            format!("{:?}", guard.state),
            guard.current_behavior.clone(),
            guard.step_index,
            guard.links.clone(),
        )
    };

    let mut lines = vec![format!("- session_id: {session_id}")];
    if !title.trim().is_empty() {
        lines.push(format!("- title: {}", title.trim()));
    }
    if !summary.trim().is_empty() {
        lines.push(format!("- summary: {}", summary.trim()));
    }

    fit_text_with_token_limit(lines.join("\n"), token_limit, tokenizer)
}

async fn load_workspace_todo_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> String {
    if token_limit == 0 {
        return String::new();
    }
    let Some(session) = input.session.as_ref() else {
        return String::new();
    };

    let (local_workspace_id, workspace_info, session_cwd) = {
        let guard = session.lock().await;
        (
            guard.local_workspace_id.clone(),
            guard.workspace_info.clone(),
            guard.cwd.clone(),
        )
    };

    let workspace_id = normalize_optional_text(local_workspace_id.as_deref())
        .or_else(|| extract_workspace_id_from_json(workspace_info.as_ref()));
    let mut lines = Vec::<String>::new();
    if let (Some(workspace_id), Some(todo_db_path)) = (
        workspace_id,
        resolve_todo_db_path(
            local_workspace_id.as_deref(),
            workspace_info.as_ref(),
            &session_cwd,
        ),
    ) {
        match render_workspace_todo_for_prompt(todo_db_path, workspace_id.clone(), token_limit)
            .await
        {
            Ok(text) if !text.trim().is_empty() => {
                lines.push(text.trim().to_string());
            }
            Ok(_) => {}
            Err(err) => {
                warn!(
                    "prompt.load_workspace_todo render failed: workspace_id={} err={}",
                    workspace_id, err
                );
            }
        }
    }

    if lines.is_empty() {
        return String::new();
    }
    fit_text_with_token_limit(lines.join("\n"), token_limit, tokenizer)
}

async fn render_workspace_todo_for_prompt(
    db_path: PathBuf,
    workspace_id: String,
    token_limit: u32,
) -> Result<String, String> {
    let token_budget = usize::try_from(token_limit).unwrap_or(usize::MAX);
    task::spawn_blocking(move || {
        render_workspace_todo_prompt_from_db(&db_path, workspace_id.as_str(), token_budget)
            .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| format!("render todo join error: {err}"))?
}

fn fit_text_with_token_limit(
    content: String,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> String {
    let trimmed = content.trim();
    if token_limit == 0 || trimmed.is_empty() {
        return String::new();
    }
    if tokenizer.count_tokens(trimmed) <= token_limit {
        return trimmed.to_string();
    }
    truncate_to_token_budget(trimmed, token_limit)
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
    let roots = collect_candidate_ancestors(&candidates);

    if let Some(local_workspace_id) = normalize_optional_text(local_workspace_id) {
        for root in &roots {
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

    for root in &roots {
        let todo_db_path = root.join("todo").join("todo.db");
        if todo_db_path.is_file() {
            return Some(todo_db_path);
        }
    }
    None
}

fn resolve_worklog_db_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    let candidates = collect_workspace_path_candidates(workspace_info, session_cwd);
    let roots = collect_candidate_ancestors(&candidates);

    if let Some(local_workspace_id) = normalize_optional_text(local_workspace_id) {
        for root in &roots {
            let local_workspace_path = root
                .join("workspaces")
                .join("local")
                .join(local_workspace_id.as_str());
            let worklog_db_path = root.join("worklog").join("worklog.db");
            if local_workspace_path.is_dir() && worklog_db_path.is_file() {
                return Some(worklog_db_path);
            }
        }
    }

    for root in &roots {
        let worklog_db_path = root.join("worklog").join("worklog.db");
        if worklog_db_path.is_file() {
            return Some(worklog_db_path);
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
            if let Some(path) = workspace_info
                .pointer(pointer)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
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

fn non_empty_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path.to_path_buf())
    }
}

async fn load_history_messages_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }

    let (session_id, workspace_info, session_cwd) = if let Some(session) = input.session.as_ref() {
        let guard = session.lock().await;
        (
            normalize_optional_text(Some(guard.session_id.as_str()))
                .or_else(|| normalize_optional_text(input.session_id.as_deref())),
            guard.workspace_info.clone(),
            guard.cwd.clone(),
        )
    } else {
        (
            normalize_optional_text(input.session_id.as_deref()),
            None,
            PathBuf::new(),
        )
    };
    let Some(session_id) = session_id else {
        return vec![];
    };

    let Some(record_file) =
        resolve_session_msg_record_path(session_id.as_str(), workspace_info.as_ref(), &session_cwd)
            .await
    else {
        return vec![];
    };

    let payload = match fs::read_to_string(&record_file).await {
        Ok(text) => text,
        Err(err) => {
            warn!(
                "prompt.load_history_messages read failed: path={} err={}",
                record_file.display(),
                err
            );
            return vec![];
        }
    };

    let lines = payload.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return vec![];
    }

    let mut records = Vec::<MemoryTimelineRecord>::new();
    let mut used_tokens = 0_u32;
    for line_idx in (0..lines.len()).rev() {
        let line = lines[line_idx].trim();
        if line.is_empty() {
            continue;
        }

        let msg_record = match serde_json::from_str::<MsgRecordWithObject>(line) {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    "prompt.load_history_messages skip invalid json line: path={} line={} err={}",
                    record_file.display(),
                    line_idx + 1,
                    err
                );
                continue;
            }
        };

        let mut text = render_history_msg_line(&msg_record);
        if text.is_empty() {
            continue;
        }
        text = fit_text_with_token_limit(text, token_limit.min(96).max(16), tokenizer);
        if text.is_empty() {
            continue;
        }

        let mut item = MemoryTimelineRecord {
            update_time: resolve_history_msg_update_time(&msg_record),
            text,
            source_label: "msg",
            source_order: 20,
        };
        let mut line_tokens = tokenizer.count_tokens(format_memory_timeline_record(&item).as_str());

        if records.is_empty() && line_tokens > token_limit {
            item.text =
                truncate_to_token_budget(item.text.as_str(), token_limit.saturating_div(2).max(1));
            line_tokens = tokenizer.count_tokens(format_memory_timeline_record(&item).as_str());
        }
        if line_tokens == 0 {
            continue;
        }
        if !records.is_empty() && used_tokens.saturating_add(line_tokens) > token_limit {
            break;
        }
        if records.is_empty() && line_tokens > token_limit {
            continue;
        }

        used_tokens = used_tokens.saturating_add(line_tokens);
        records.push(item);
        if used_tokens >= token_limit {
            break;
        }
    }

    records
}

async fn resolve_session_msg_record_path(
    session_id: &str,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }

    if session_id.contains('/') || session_id.contains('\\') {
        let session_path = PathBuf::from(session_id);
        if session_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("jsonl"))
            .unwrap_or(false)
        {
            push_unique_pathbuf(&mut candidates, session_path);
        } else {
            for file_name in SESSION_MSG_RECORD_FILES {
                push_unique_pathbuf(&mut candidates, session_path.join(file_name));
            }
        }
    }

    let roots = collect_candidate_ancestors(&collect_workspace_path_candidates(
        workspace_info,
        session_cwd,
    ));
    for root in roots {
        for file_name in SESSION_MSG_RECORD_FILES {
            push_unique_pathbuf(
                &mut candidates,
                root.join("session").join(session_id).join(file_name),
            );
            push_unique_pathbuf(&mut candidates, root.join(session_id).join(file_name));
        }
    }
    for file_name in SESSION_MSG_RECORD_FILES {
        push_unique_pathbuf(
            &mut candidates,
            PathBuf::from("session").join(session_id).join(file_name),
        );
        push_unique_pathbuf(&mut candidates, PathBuf::from(session_id).join(file_name));
    }

    for candidate in candidates {
        if fs::metadata(&candidate)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

fn resolve_history_msg_update_time(record: &MsgRecordWithObject) -> u64 {
    let obj_ts = record
        .msg
        .as_ref()
        .map(|msg| msg.created_at_ms)
        .unwrap_or(0);
    record
        .record
        .updated_at_ms
        .max(record.record.created_at_ms)
        .max(obj_ts)
}

fn render_history_msg_line(record: &MsgRecordWithObject) -> String {
    let direction = match record.record.box_kind {
        BoxKind::Outbox | BoxKind::TunnelOutbox => "out",
        _ => "in",
    };

    let mut content = record
        .msg
        .as_ref()
        .map(|msg| msg.content.content.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(compact_one_line_text)
        .unwrap_or_default();
    if content.is_empty() {
        content = format!(
            "msg_id={} state={:?}",
            record.record.msg_id, record.record.state
        );
    }

    format!(
        "{} {:?} {:?} -> {:?} {}",
        direction, record.record.msg_kind, record.record.from, record.record.to, content
    )
}

fn compact_one_line_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_worklog_record_type(record_type: &str) -> String {
    let trimmed = record_type.trim();
    let without_prefix = trimmed.strip_prefix("opendan.worklog.").unwrap_or(trimmed);
    let without_version = without_prefix.strip_suffix(".v1").unwrap_or(without_prefix);
    without_version.to_string()
}

fn render_workspace_worklog_line(record: &WorklogRecord) -> String {
    let record_type = normalize_worklog_record_type(record.record_type.as_str());
    let digest = record
        .prompt_view
        .as_ref()
        .map(|view| view.digest.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            record
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(compact_one_line_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| compact_one_line_text(sanitize_json_compact(&record.payload).as_str()));
    if digest.is_empty() {
        return format!("{} status={}", record_type, record.status);
    }
    format!("{} status={} {}", record_type, record.status, digest)
}

async fn load_workspace_worklog_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }

    let (session_id, local_workspace_id, workspace_info, session_cwd) =
        if let Some(session) = input.session.as_ref() {
            let guard = session.lock().await;
            (
                normalize_optional_text(Some(guard.session_id.as_str()))
                    .or_else(|| normalize_optional_text(input.session_id.as_deref())),
                guard.local_workspace_id.clone(),
                guard.workspace_info.clone(),
                guard.cwd.clone(),
            )
        } else {
            (
                normalize_optional_text(input.session_id.as_deref()),
                None,
                None,
                PathBuf::new(),
            )
        };
    let workspace_id = normalize_optional_text(local_workspace_id.as_deref())
        .or_else(|| extract_workspace_id_from_json(workspace_info.as_ref()));
    if session_id.is_none() && workspace_id.is_none() {
        return vec![];
    }

    let Some(worklog_db_path) = resolve_worklog_db_path(
        local_workspace_id.as_deref(),
        workspace_info.as_ref(),
        &session_cwd,
    ) else {
        return vec![];
    };

    let query_limit = usize::try_from(token_limit.saturating_mul(2))
        .unwrap_or(usize::MAX)
        .clamp(16, 256);
    let worklog_tool =
        match WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db_path.clone())) {
            Ok(tool) => tool,
            Err(err) => {
                warn!(
                    "prompt.load_workspace_worklog create tool failed: path={} err={}",
                    worklog_db_path.display(),
                    err
                );
                return vec![];
            }
        };

    let worklog_records = match worklog_tool
        .list_worklog_records(WorklogListOptions {
            owner_session_id: session_id,
            workspace_id,
            limit: Some(query_limit),
            ..Default::default()
        })
        .await
    {
        Ok(records) => records,
        Err(err) => {
            warn!(
                "prompt.load_workspace_worklog list failed: path={} err={}",
                worklog_db_path.display(),
                err
            );
            return vec![];
        }
    };
    if worklog_records.is_empty() {
        return vec![];
    }

    let mut records = Vec::<MemoryTimelineRecord>::new();
    let mut used_tokens = 0_u32;
    for worklog_record in worklog_records {
        if worklog_record.commit_state.eq_ignore_ascii_case("PENDING") {
            continue;
        }

        let mut text = render_workspace_worklog_line(&worklog_record);
        if text.is_empty() {
            continue;
        }
        text = fit_text_with_token_limit(text, token_limit.min(128).max(24), tokenizer);
        if text.is_empty() {
            continue;
        }

        let mut item = MemoryTimelineRecord {
            update_time: worklog_record.timestamp,
            text,
            source_label: "worklog",
            source_order: 30,
        };
        let mut line_tokens = tokenizer.count_tokens(format_memory_timeline_record(&item).as_str());

        if records.is_empty() && line_tokens > token_limit {
            item.text =
                truncate_to_token_budget(item.text.as_str(), token_limit.saturating_div(2).max(1));
            line_tokens = tokenizer.count_tokens(format_memory_timeline_record(&item).as_str());
        }
        if line_tokens == 0 {
            continue;
        }
        if !records.is_empty() && used_tokens.saturating_add(line_tokens) > token_limit {
            break;
        }
        if records.is_empty() && line_tokens > token_limit {
            continue;
        }

        used_tokens = used_tokens.saturating_add(line_tokens);
        records.push(item);
        if used_tokens >= token_limit {
            break;
        }
    }

    records
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct ToolboxSkillRecord {
    name: String,
    introduce: String,
}

#[derive(Clone, Debug, Default)]
struct ToolboxSkillSpec {
    introduce: String,
    actions: Vec<String>,
    loaded_tools: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
struct RawToolboxSkillSpec {
    name: String,
    introduce: String,
    actions: Vec<String>,
    loaded_tools: Vec<String>,
}

impl RawToolboxSkillSpec {
    fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.introduce = self.introduce.trim().to_string();
        self.actions = normalize_unique_string_list(std::mem::take(&mut self.actions));
        self.loaded_tools = normalize_unique_string_list(std::mem::take(&mut self.loaded_tools));
    }

    fn to_toolbox_skill_spec(self) -> ToolboxSkillSpec {
        ToolboxSkillSpec {
            introduce: self.introduce,
            actions: self.actions,
            loaded_tools: self.loaded_tools,
        }
    }
}

//构成的提示词为
// 列出的workspace的skill Record列表
// session里得到当前加载的skills（提示词注入)
// 所有可用的action定义（通过加载的所有skills的actions查找）
//生成的Vec<AiToolSpec>:
// 当前加载的tools的定义 （合并所有已经加载的skills的tool定义)
async fn build_toolbox(
    tools: &[AiToolSpec],
    action_specs: &[ActionSpec],
    cfg: &BehaviorConfig,
    session: Option<Arc<Mutex<AgentSession>>>,
) -> Option<(String, Vec<AiToolSpec>)> {
    if cfg.toolbox.is_none_mode() {
        return None;
    }

    let mut session_loaded_skills = Vec::<String>::new();
    let mut local_workspace_id: Option<String> = None;
    let mut workspace_info: Option<Json> = None;
    let mut session_cwd = PathBuf::new();
    if let Some(session) = session.as_ref() {
        let guard = session.lock().await;
        session_loaded_skills = normalize_unique_string_list(guard.loaded_skills.clone());
        local_workspace_id = normalize_optional_text(guard.local_workspace_id.as_deref());
        workspace_info = guard.workspace_info.clone();
        session_cwd = guard.cwd.clone();
    }

    let (workspace_skill_records, workspace_skill_specs) = load_workspace_skill_catalog(
        local_workspace_id.as_deref(),
        workspace_info.as_ref(),
        &session_cwd,
    )
    .await;

    let behavior_skills = cfg.toolbox.effective_skills();
    let loaded_skills = merge_unique_string_slices(&behavior_skills, &session_loaded_skills);

    let mut actions = normalize_unique_string_list(cfg.toolbox.default_load_actions.clone());
    let mut loaded_tool_names = Vec::<String>::new();
    for skill_name in &loaded_skills {
        let Some(spec) = workspace_skill_specs.get(skill_name.as_str()) else {
            continue;
        };
        actions = merge_unique_string_slices(&actions, &spec.actions);
        loaded_tool_names = merge_unique_string_slices(&loaded_tool_names, &spec.loaded_tools);
    }

    let filtered_tools = cfg.toolbox.tools.filter_ai_tool_specs(tools);
    let merged_tools = merge_tool_specs_with_skill_tools(filtered_tools, &loaded_tool_names, tools);

    if merged_tools.is_empty()
        && loaded_skills.is_empty()
        && workspace_skill_records.is_empty()
        && actions.is_empty()
    {
        return None;
    }

    let selected_action_specs = select_action_specs(action_specs, &actions);
    let selected_action_prompts = selected_action_specs
        .iter()
        .map(|spec| spec.render_prompt())
        .collect::<Vec<_>>();
    let value = json!({
        "workspace_skill_records": workspace_skill_records,
        "loaded_skills": loaded_skills,
        "allow_actions": actions,
        "actions": actions,
        "action_specs": selected_action_specs,
        "action_prompts": selected_action_prompts,
    });
    Some((sanitize_json_compact(&value), merged_tools))
}

fn select_action_specs(action_specs: &[ActionSpec], action_names: &[String]) -> Vec<ActionSpec> {
    if action_specs.is_empty() || action_names.is_empty() {
        return vec![];
    }
    let mut by_name = HashMap::<String, ActionSpec>::new();
    for spec in action_specs {
        let normalized = normalize_tool_name(spec.name.as_str());
        if normalized.is_empty() {
            continue;
        }
        by_name.insert(normalized, spec.clone());
    }

    let mut selected = Vec::<ActionSpec>::new();
    for action_name in action_names {
        let normalized = normalize_tool_name(action_name.as_str());
        if normalized.is_empty() {
            continue;
        }
        if let Some(spec) = by_name.get(normalized.as_str()) {
            selected.push(spec.clone());
        }
    }
    selected
}

fn merge_tool_specs_with_skill_tools(
    mut base: Vec<AiToolSpec>,
    loaded_tool_names: &[String],
    all_tools: &[AiToolSpec],
) -> Vec<AiToolSpec> {
    let mut seen = base
        .iter()
        .map(|item| item.name.clone())
        .collect::<HashSet<_>>();

    for tool_name in loaded_tool_names {
        let normalized = tool_name.trim();
        if normalized.is_empty() || !seen.insert(normalized.to_string()) {
            continue;
        }
        if let Some(spec) = all_tools.iter().find(|item| item.name == normalized) {
            base.push(spec.clone());
        }
    }
    base
}

fn merge_unique_string_slices(primary: &[String], secondary: &[String]) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut uniq = HashSet::<String>::new();
    for value in primary.iter().chain(secondary.iter()) {
        let normalized = value.trim();
        if normalized.is_empty() {
            continue;
        }
        if uniq.insert(normalized.to_string()) {
            out.push(normalized.to_string());
        }
    }
    out
}

fn normalize_unique_string_list(values: Vec<String>) -> Vec<String> {
    merge_unique_string_slices(&values, &[])
}

async fn load_workspace_skill_catalog(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> (Vec<ToolboxSkillRecord>, HashMap<String, ToolboxSkillSpec>) {
    let skill_roots =
        collect_workspace_skill_roots(local_workspace_id, workspace_info, session_cwd).await;
    if skill_roots.is_empty() {
        return (vec![], HashMap::new());
    }

    let mut records = HashMap::<String, ToolboxSkillRecord>::new();
    let mut specs = HashMap::<String, ToolboxSkillSpec>::new();
    for skills_root in skill_roots {
        merge_skill_catalog_from_root(skills_root.as_path(), &mut records, &mut specs).await;
    }

    let mut workspace_records = records.into_values().collect::<Vec<_>>();
    workspace_records.sort_by(|left, right| left.name.cmp(&right.name));
    (workspace_records, specs)
}

async fn collect_workspace_skill_roots(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    if let Some(workspace_info) = workspace_info {
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
            if let Some(path) = workspace_info
                .pointer(pointer)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                push_unique_pathbuf(&mut candidates, PathBuf::from(path));
            }
        }
    }
    if let Some(path) = non_empty_path(session_cwd) {
        push_unique_pathbuf(&mut candidates, path);
    }
    if candidates.is_empty() {
        return vec![];
    }

    let roots = collect_candidate_ancestors(&candidates);
    let mut skill_roots = Vec::<PathBuf>::new();
    for root in roots {
        push_unique_pathbuf(&mut skill_roots, root.join("skills"));
        if let Some(local_workspace_id) = normalize_optional_text(local_workspace_id) {
            push_unique_pathbuf(
                &mut skill_roots,
                root.join("workspaces")
                    .join("local")
                    .join(local_workspace_id)
                    .join("skills"),
            );
        }
    }

    let mut existing = Vec::<PathBuf>::new();
    for skill_root in skill_roots {
        if fs::metadata(&skill_root)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            existing.push(skill_root);
        }
    }
    existing
}

async fn merge_skill_catalog_from_root(
    skills_root: &Path,
    records: &mut HashMap<String, ToolboxSkillRecord>,
    specs: &mut HashMap<String, ToolboxSkillSpec>,
) {
    let mut entries = match fs::read_dir(skills_root).await {
        Ok(value) => value,
        Err(err) => {
            warn!(
                "prompt.load_workspace_skills read_dir failed: path={} err={}",
                skills_root.display(),
                err
            );
            return;
        }
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let is_dir = entry
            .file_type()
            .await
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false);
        if !is_dir {
            continue;
        }

        let Some(skill_key) = entry
            .file_name()
            .to_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
        else {
            continue;
        };

        let Some(skill_file_path) = find_skill_spec_file(skills_root, skill_key.as_str()).await
        else {
            continue;
        };

        let Some((raw_name, spec)) = read_skill_spec(skill_file_path.as_path()).await else {
            continue;
        };
        let display_name = normalize_optional_text(Some(raw_name.as_str())).unwrap_or(skill_key);
        records
            .entry(display_name.clone())
            .or_insert(ToolboxSkillRecord {
                name: display_name.clone(),
                introduce: spec.introduce.clone(),
            });

        specs
            .entry(display_name.clone())
            .or_insert_with(|| spec.clone());
        specs.entry(raw_name).or_insert(spec);
    }
}

async fn find_skill_spec_file(skills_root: &Path, skill_name: &str) -> Option<PathBuf> {
    let skill_name = skill_name.trim();
    if skill_name.is_empty() {
        return None;
    }
    let skill_dir = skills_root.join(skill_name);
    if !fs::metadata(&skill_dir)
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
    {
        return None;
    }

    for ext in SKILL_SPEC_EXTENSIONS {
        let file_path = skill_dir.join(format!("{skill_name}.{ext}"));
        if fs::metadata(&file_path)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false)
        {
            return Some(file_path);
        }
    }
    None
}

async fn read_skill_spec(path: &Path) -> Option<(String, ToolboxSkillSpec)> {
    let raw_content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(err) => {
            warn!(
                "prompt.load_workspace_skills read failed: path={} err={}",
                path.display(),
                err
            );
            return None;
        }
    };

    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    let mut spec =
        match ext.as_deref() {
            Some("json") => serde_json::from_str::<RawToolboxSkillSpec>(&raw_content)
                .map_err(|err| err.to_string()),
            Some("yaml") | Some("yml") => serde_yaml::from_str::<RawToolboxSkillSpec>(&raw_content)
                .map_err(|err| err.to_string()),
            _ => {
                let json_try = serde_json::from_str::<RawToolboxSkillSpec>(&raw_content)
                    .map_err(|err| err.to_string());
                match json_try {
                    Ok(spec) => Ok(spec),
                    Err(json_err) => serde_yaml::from_str::<RawToolboxSkillSpec>(&raw_content)
                        .map_err(|yaml_err| format!("json={json_err}; yaml={yaml_err}")),
                }
            }
        }
        .map_err(|err| {
            warn!(
                "prompt.load_workspace_skills parse failed: path={} err={}",
                path.display(),
                err
            );
            err
        })
        .ok()?;

    spec.normalize();
    let display_name = if spec.name.is_empty() {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
            .unwrap_or_default()
    } else {
        spec.name.clone()
    };
    Some((display_name, spec.to_toolbox_skill_spec()))
}

fn contains_any_marker(content: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| content.contains(marker))
}

pub struct Truncator;

impl Truncator {
    pub fn fit_into_budget(
        mut messages: Vec<ChatMessage>,
        max_prompt_tokens: u32,
        tokenizer: &dyn Tokenizer,
    ) -> Vec<ChatMessage> {
        if max_prompt_tokens == 0 {
            return vec![];
        }

        let mut total = message_tokens(&messages, tokenizer);
        if total <= max_prompt_tokens {
            return messages;
        }

        let obs_idx = messages.iter().position(|m| {
            contains_any_marker(
                m.content.as_str(),
                &["<<Observations>>", "<<OBSERVATIONS>>", "<<observations>>"],
            )
        });
        if let Some(idx) = obs_idx {
            messages.remove(idx);
            total = message_tokens(&messages, tokenizer);
            if total <= max_prompt_tokens {
                return messages;
            }
        }

        let memory_idx = messages.iter().position(|m| {
            contains_any_marker(
                m.content.as_str(),
                &["<<Memory>>", "<<MEMORY>>", "<<memory>>"],
            )
        });
        if let Some(idx) = memory_idx {
            messages.remove(idx);
            total = message_tokens(&messages, tokenizer);
            if total <= max_prompt_tokens {
                return messages;
            }
        }

        let shrink_order = ["<<Input>>", "<<toolbox>>", "<<role>>"];
        for marker in shrink_order {
            let Some(idx) = messages.iter().position(|m| m.content.contains(marker)) else {
                continue;
            };
            let current_total = message_tokens(&messages, tokenizer);
            if current_total <= max_prompt_tokens {
                break;
            }
            let msg_tokens = tokenizer.count_tokens(&messages[idx].content);
            let overflow = current_total.saturating_sub(max_prompt_tokens);
            let keep_tokens = msg_tokens.saturating_sub(overflow).max(32);
            messages[idx].content = truncate_to_token_budget(&messages[idx].content, keep_tokens);
        }

        while !messages.is_empty() && message_tokens(&messages, tokenizer) > max_prompt_tokens {
            let last = messages.len() - 1;
            let msg_tokens = tokenizer.count_tokens(&messages[last].content);
            let keep_tokens = msg_tokens / 2;
            if keep_tokens < 8 {
                messages.remove(last);
                continue;
            }
            messages[last].content = truncate_to_token_budget(&messages[last].content, keep_tokens);
        }

        messages
    }
}

fn truncate_to_token_budget(content: &str, keep_tokens: u32) -> String {
    if keep_tokens == 0 {
        return "[TRUNCATED]".to_string();
    }

    let mut out = String::new();
    for (idx, token) in content.split_whitespace().enumerate() {
        if idx as u32 >= keep_tokens {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
    }

    if out.is_empty() {
        let chars = content
            .chars()
            .take(keep_tokens as usize * 4)
            .collect::<String>();
        return format!("{chars} [TRUNCATED]");
    }

    format!("{out} [TRUNCATED]")
}

fn message_tokens(messages: &[ChatMessage], tokenizer: &dyn Tokenizer) -> u32 {
    messages
        .iter()
        .map(|m| tokenizer.count_tokens(&m.content))
        .sum()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
    use crate::agent_session::AgentSession;
    use crate::agent_tool::AgentTool;
    use crate::behavior::types::{StepLimits, TraceCtx};
    use crate::worklog::{WorklogTool, WorklogToolConfig};
    use buckyos_api::MsgRecordWithObject;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    struct MockTokenizer;

    impl Tokenizer for MockTokenizer {
        fn count_tokens(&self, text: &str) -> u32 {
            text.split_whitespace().count() as u32
        }
    }

    #[tokio::test]
    async fn build_toolbox_loads_workspace_skills_and_merges_action_tool_defs() {
        let temp = tempdir().expect("create tempdir");
        let workspace_root = temp.path().join("workspace");
        let skills_root = workspace_root.join("skills").join("coding");
        tokio::fs::create_dir_all(&skills_root)
            .await
            .expect("create skills dir");
        tokio::fs::write(
            skills_root.join("coding.yaml"),
            r#"
name: coding
introduce: coding skill
actions: [build, test]
loaded_tools: [exec_bash]
"#,
        )
        .await
        .expect("write skill spec");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.cwd = workspace_root;
        session.loaded_skills = vec!["coding".to_string()];
        let session = Arc::new(Mutex::new(session));

        let mut cfg = BehaviorConfig::default();
        cfg.toolbox.tools.mode = crate::behavior::config::BehaviorToolMode::AllowList;
        cfg.toolbox.tools.names = vec!["read_file".to_string()];
        cfg.toolbox.default_load_actions = vec!["lint".to_string()];

        let tools = vec![
            AiToolSpec {
                name: "read_file".to_string(),
                description: "read file".to_string(),
                args_schema: HashMap::new(),
                output_schema: json!({"type":"object"}),
            },
            AiToolSpec {
                name: "exec_bash".to_string(),
                description: "exec bash".to_string(),
                args_schema: HashMap::new(),
                output_schema: json!({"type":"object"}),
            },
        ];
        let action_specs = vec![
            ActionSpec {
                kind: crate::agent_tool::ActionKind::CallTool,
                name: "build".to_string(),
                introduce: "build project".to_string(),
                description: Some("run build workflow".to_string()),
            },
            ActionSpec {
                kind: crate::agent_tool::ActionKind::CallTool,
                name: "test".to_string(),
                introduce: "run tests".to_string(),
                description: Some("run test workflow".to_string()),
            },
        ];

        let (toolbox_text, loaded_tools) =
            build_toolbox(&tools, &action_specs, &cfg, Some(session))
                .await
                .expect("toolbox should be available");
        let toolbox_json: Json =
            serde_json::from_str(&toolbox_text).expect("toolbox should be valid json");

        let record_names = toolbox_json["workspace_skill_records"]
            .as_array()
            .expect("workspace_skill_records should be array")
            .iter()
            .filter_map(|item| item.get("name"))
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>();
        assert_eq!(record_names, vec!["coding"]);

        let loaded_skills = toolbox_json["loaded_skills"]
            .as_array()
            .expect("loaded_skills should be array")
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>();
        assert_eq!(loaded_skills, vec!["coding"]);

        let actions = toolbox_json["actions"]
            .as_array()
            .expect("actions should be array")
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actions, vec!["lint", "build", "test"]);
        let action_prompts = toolbox_json["action_prompts"]
            .as_array()
            .expect("action_prompts should be array")
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>();
        assert_eq!(action_prompts.len(), 2);

        let tool_names = loaded_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(tool_names, vec!["read_file", "exec_bash"]);
    }

    #[tokio::test]
    async fn build_produces_valid_prompt() {
        let input = BehaviorExecInput {
            session_id: Some("session-1".to_string()),
            trace: TraceCtx {
                trace_id: "trace-1".to_string(),
                agent_did: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 2,
                wakeup_id: "wakeup-1".to_string(),
                session_id: None,
            },
            input_prompt: "user input".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a helpful assistant.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: "rules".to_string(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig {
                process_rule: "Process rules.".to_string(),
                policy: "Policy text.".to_string(),
                ..Default::default()
            },
            session: None,
        };

        let req = PromptBuilder::build(
            &input,
            &[],
            &[],
            &input.behavior_cfg,
            &MockTokenizer,
            None,
            None,
        )
        .await
        .expect("build prompt");

        let system = req
            .payload
            .messages
            .first()
            .map(|msg| msg.content.clone())
            .unwrap_or_default();

        assert!(system.contains("<<role>>"));
        assert!(system.contains("<<process_rules>>"));
        assert!(system.contains("<<policy>>"));
        assert!(system.contains("<<output_protocol>>"));
        assert!(system.contains("You are a helpful assistant."));
        assert!(system.contains("Process rules."));
    }

    #[tokio::test]
    async fn load_agent_memory_with_limit_reads_from_memory_module() {
        let temp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(temp.path()))
            .await
            .expect("create agent memory");
        memory
            .set_memory(
                "/user/preference/style",
                json!({
                    "type":"preference",
                    "summary":"用户偏好简洁回复",
                    "importance": 7,
                    "tags": ["style"]
                }),
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-02-22T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m1"}
                }),
            )
            .await
            .expect("set memory");

        let input = BehaviorExecInput {
            session_id: Some("session-1".to_string()),
            trace: TraceCtx {
                trace_id: "trace-1".to_string(),
                agent_did: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: None,
            },
            input_prompt: "user input".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a helpful assistant.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: "rules".to_string(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig::default(),
            session: None,
        };

        let records = load_agent_memory_with_limit(
            &input,
            &["style".to_string()],
            200,
            &MockTokenizer,
            Some(memory),
        )
        .await;

        assert!(!records.is_empty(), "expected memory timeline records");
        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("user/preference/style"));
        assert!(merged.contains("用户偏好简洁回复"));
    }

    #[tokio::test]
    async fn load_history_messages_with_limit_reads_session_jsonl_reverse() {
        let temp = tempdir().expect("create tempdir");
        let workspace_root = temp.path().join("workspace");
        let session_id = "session-1";
        let session_dir = workspace_root.join("session").join(session_id);
        tokio::fs::create_dir_all(&session_dir)
            .await
            .expect("create session dir");

        let line1 = json!({
            "record": {
                "record_id": "r1",
                "box_kind": "INBOX",
                "msg_id": "sha256:11111111111111111111111111111111",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "to": "did:web:bob.example.com",
                "created_at_ms": 1000,
                "updated_at_ms": 1000,
                "sort_key": 1000,
                "tags": []
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:bob.example.com"],
                "kind": "chat",
                "created_at_ms": 1000,
                "content": {
                    "content": "hello from line1"
                }
            }
        });
        let line2 = json!({
            "record": {
                "record_id": "r2",
                "box_kind": "INBOX",
                "msg_id": "sha256:22222222222222222222222222222222",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "to": "did:web:bob.example.com",
                "created_at_ms": 2000,
                "updated_at_ms": 2000,
                "sort_key": 2000,
                "tags": []
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:bob.example.com"],
                "kind": "chat",
                "created_at_ms": 2000,
                "content": {
                    "content": "hello from line2"
                }
            }
        });

        tokio::fs::write(
            session_dir.join("msg_record.jsonl"),
            format!("{line1}\n{line2}\n"),
        )
        .await
        .expect("write msg record file");

        let mut session = AgentSession::new(session_id, "did:web:agent.example.com", None);
        session.cwd = workspace_root.clone();
        let input = BehaviorExecInput {
            session_id: Some(session_id.to_string()),
            trace: TraceCtx {
                trace_id: "trace-1".to_string(),
                agent_did: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: None,
            },
            input_prompt: "user input".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a helpful assistant.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: "rules".to_string(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig::default(),
            session: Some(Arc::new(Mutex::new(session))),
        };

        let records = load_history_messages_with_limit(&input, 200, &MockTokenizer).await;
        assert!(!records.is_empty(), "expected history message records");
        let first = records.first().expect("first record");
        assert_eq!(first.source_label, "msg");
        assert!(
            first.text.contains("line2"),
            "expected reverse-read newest line first, got: {}",
            first.text
        );

        for line in [line1.to_string(), line2.to_string()] {
            serde_json::from_str::<MsgRecordWithObject>(&line).expect("line should parse");
        }
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_reads_from_worklog_db() {
        let temp = tempdir().expect("create tempdir");
        let workspace_root = temp.path().join("workspace");
        let worklog_db = workspace_root.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = TraceCtx {
            trace_id: "trace-worklog".to_string(),
            agent_did: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: None,
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "opendan.worklog.FunctionRecord.v1",
                        "owner_session_id": "session-1",
                        "status": "OK",
                        "prompt_view": {
                            "digest": "digest_session_1",
                            "detail": { "kind": "test" }
                        },
                        "payload": {
                            "tool_name": "read_file"
                        }
                    }
                }),
            )
            .await
            .expect("append worklog for session-1");
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "opendan.worklog.FunctionRecord.v1",
                        "owner_session_id": "session-2",
                        "status": "OK",
                        "prompt_view": {
                            "digest": "digest_session_2",
                            "detail": { "kind": "test" }
                        },
                        "payload": {
                            "tool_name": "write_file"
                        }
                    }
                }),
            )
            .await
            .expect("append worklog for session-2");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.cwd = workspace_root;
        let input = BehaviorExecInput {
            session_id: Some("session-1".to_string()),
            trace: TraceCtx {
                trace_id: "trace-1".to_string(),
                agent_did: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: None,
            },
            input_prompt: "user input".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a helpful assistant.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: "rules".to_string(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig::default(),
            session: Some(Arc::new(Mutex::new(session))),
        };

        let records = load_workspace_worklog_with_limit(&input, 200, &MockTokenizer).await;
        assert!(
            !records.is_empty(),
            "expected workspace worklog timeline records"
        );
        assert_eq!(records[0].source_label, "worklog");

        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("digest_session_1"));
        assert!(!merged.contains("digest_session_2"));
    }
}
