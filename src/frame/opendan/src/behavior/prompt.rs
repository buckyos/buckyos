use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{
    features, AiMessage, AiPayload, BoxKind, Capability, CompleteRequest, ModelSpec,
    MsgRecordWithObject, Requirements,
};
use chrono::{DateTime, Utc};
use log::{debug, warn};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::fs;
use tokio::sync::Mutex;

use crate::agent_environment::AgentEnvironment;
use crate::agent_session::AgentSession;
use crate::agent_tool::AgentMemory;
use crate::behavior::config::BehaviorMemoryBucketConfig;
use crate::behavior::BehaviorConfig;
use crate::worklog::{
    render_worklog_prompt_line, WorklogListOptions, WorklogRecord, WorklogRecordType,
    WorklogService, WorklogToolConfig,
};
use crate::workspace::agent_skill::{
    load_skill_from_root, merge_skill_records_from_dir, AgentSkillRecord,
};
use crate::workspace_path::{
    resolve_agent_env_root,
    resolve_default_local_workspace_path as resolve_default_local_workspace_root,
    resolve_session_workspace_root, LOCAL_WORKSPACE_SKILLS_DIR,
    LOCAL_WORKSPACE_WORKLOG_DB_REL_PATH, WORKSHOP_WORKLOG_DB_REL_PATH,
};

use super::sanitize::{sanitize_json_compact, sanitize_text};
use super::types::BehaviorExecInput;
use super::Tokenizer;

const SESSION_MSG_RECORD_FILES: [&str; 2] = ["msg_record.jsonl", "message_record.jsonl"];

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
        cfg: &BehaviorConfig,
        tokenizer: &dyn Tokenizer,
        session: Option<Arc<Mutex<AgentSession>>>,
        memory: Option<AgentMemory>,
    ) -> Result<CompleteRequest, String> {
        let env_context = build_env_context(input);
        let loaded_tools = Vec::new();

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

        // 根据当前session加载的skills，空内容时跳过<<skills>> section
        let skills_text = render_skills_text(session.clone()).await?;
        if !skills_text.trim().is_empty() {
            system_parts.push(format!(
                "<<skills>>\n{}\n<</skills>>",
                sanitize_text(skills_text.as_str())
            ));
        }

        system_parts.push(format!(
            "<<output_protocol>>\n{}\n<</output_protocol>>",
            sanitize_text(output_protocol_text.as_str())
        ));

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

        let mut must_features = Vec::new();
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
    if !input.session_id.trim().is_empty() {
        ctx.insert(
            "session_id".to_string(),
            Json::String(input.session_id.clone()),
        );
        ctx.insert(
            "loop.session_id".to_string(),
            Json::String(input.session_id.clone()),
        );
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

async fn render_skills_text(session: Option<Arc<Mutex<AgentSession>>>) -> Result<String, String> {
    let Some(session) = session else {
        return Ok(String::new());
    };

    let (loaded_skills, local_workspace_id, workspace_info, session_cwd) = {
        let guard = session.lock().await;
        (
            guard.loaded_skills.clone(),
            guard.local_workspace_id.clone(),
            guard.workspace_info.clone(),
            guard.pwd.clone(),
        )
    };

    let skill_roots = collect_workspace_skill_roots(
        local_workspace_id.as_deref(),
        workspace_info.as_ref(),
        &session_cwd,
    )
    .await;

    let mut all_records = HashMap::<String, AgentSkillRecord>::new();
    for root in &skill_roots {
        let _ = merge_skill_records_from_dir(root.as_path(), &mut all_records).await;
    }

    let mut loaded_rules = Vec::<String>::new();
    let loaded_skills = normalize_unique_string_list(loaded_skills);
    for skill_name in &loaded_skills {
        for root in &skill_roots {
            if let Ok(spec) = load_skill_from_root(root.as_path(), skill_name.as_str()).await {
                if !spec.rules.is_empty() {
                    loaded_rules.push(format!("## {} Skill\n{}", skill_name, spec.rules));
                }
                break;
            }
        }
    }

    Ok(loaded_rules.join("\n\n"))
}

fn normalize_unique_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for value in values {
        let normalized = value.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            out.push(normalized.to_string());
        }
    }
    out
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

    let env_context = build_env_context(input);
    let memory_head = render_memory_boundary_text(
        cfg.memory.first_prompt.as_deref(),
        &env_context,
        input.session.clone(),
    )
    .await;
    let memory_tail = render_memory_boundary_text(
        cfg.memory.last_prompt.as_deref(),
        &env_context,
        input.session.clone(),
    )
    .await;
    let fixed_prompt = compose_memory_prompt(memory_head.as_str(), "", memory_tail.as_str());
    let fixed_budget = if fixed_prompt.is_empty() {
        0
    } else {
        tokenizer.count_tokens(fixed_prompt.as_str())
    };
    let dynamic_budget = total_budget.saturating_sub(fixed_budget);

    let workspace_summary_budget =
        calc_memory_bucket_budget(dynamic_budget, &cfg.memory.workspace_summary);
    let agent_memory_budget = calc_memory_bucket_budget(dynamic_budget, &cfg.memory.agent_memory);
    let history_messages_budget =
        calc_memory_bucket_budget(dynamic_budget, &cfg.memory.history_messages);
    let workspace_worklog_budget =
        calc_memory_bucket_budget(dynamic_budget, &cfg.memory.workspace_worklog);
    let session_summaries_budget =
        calc_memory_bucket_budget(dynamic_budget, &cfg.memory.session_summaries);

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

    if agent_memory_budget > 0 {
        let remembered_things = load_agent_memory_with_limit(
            input,
            topic_tags.as_slice(),
            agent_memory_budget,
            tokenizer,
            memory.clone(),
        )
        .await;
        if !remembered_things.trim().is_empty() {
            memory_sections.push(format!(
                "## What you remember:\n{}\n",
                sanitize_text(remembered_things.trim())
            ));
        }
    }

    let mut timeline_records = Vec::<MemoryTimelineRecord>::new();
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
        let timeline_text = render_memory_timeline_text(&timeline_records);
        memory_sections.push(format!(
            "## Timeline Logs\n```log\n{}\n```\n",
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

    if !session_summary_parts.is_empty() {
        memory_sections.push(session_summary_parts.join("\n"));
    }

    let mut memory_body = if dynamic_budget == 0 {
        String::new()
    } else {
        fit_text_with_token_limit(memory_sections.join("\n\n"), dynamic_budget, tokenizer)
    };
    let mut assembled = compose_memory_prompt(
        memory_head.as_str(),
        memory_body.as_str(),
        memory_tail.as_str(),
    );
    if assembled.is_empty() {
        return String::new();
    }

    // Header/footer templates are fixed text and should never be truncated.
    // If token count still exceeds budget, only shrink dynamic memory body.
    let mut guard = 0_u8;
    while !memory_body.is_empty()
        && tokenizer.count_tokens(assembled.as_str()) > total_budget
        && guard < 6
    {
        let overflow = tokenizer
            .count_tokens(assembled.as_str())
            .saturating_sub(total_budget);
        let body_tokens = tokenizer.count_tokens(memory_body.as_str());
        if body_tokens <= overflow {
            memory_body.clear();
        } else {
            memory_body = truncate_to_token_budget(
                memory_body.as_str(),
                body_tokens.saturating_sub(overflow),
            );
        }
        assembled = compose_memory_prompt(
            memory_head.as_str(),
            memory_body.as_str(),
            memory_tail.as_str(),
        );
        guard = guard.saturating_add(1);
    }
    assembled
}

async fn render_memory_boundary_text(
    template: Option<&str>,
    env_context: &HashMap<String, Json>,
    session: Option<Arc<Mutex<AgentSession>>>,
) -> String {
    let Some(template) = template.map(str::trim).filter(|text| !text.is_empty()) else {
        return String::new();
    };

    let rendered = match render_section(template, env_context, session).await {
        Ok(text) => text,
        Err(err) => {
            warn!("prompt.render_memory_boundary_text failed: {}", err);
            template.to_string()
        }
    };
    sanitize_text(rendered.trim())
}

fn compose_memory_prompt(memory_head: &str, memory_body: &str, memory_tail: &str) -> String {
    let mut parts = Vec::<String>::new();
    if !memory_head.trim().is_empty() {
        parts.push(memory_head.trim().to_string());
    }
    if !memory_body.trim().is_empty() {
        parts.push(memory_body.trim().to_string());
    }
    if !memory_tail.trim().is_empty() {
        parts.push(memory_tail.trim().to_string());
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("<<memory>>\n{}\n<</memory>>", parts.join("\n\n"))
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

    line.to_string()
}

fn render_memory_timeline_text(records: &[MemoryTimelineRecord]) -> String {
    let mut lines = Vec::<String>::new();
    let mut last_msg_day: Option<String> = None;
    for record in records {
        let text = record.text.trim();
        if text.is_empty() {
            continue;
        }
        if record.source_label == "msg" {
            let day = format_history_date(record.update_time);
            if last_msg_day.as_deref() != Some(day.as_str()) {
                if !lines.is_empty() && lines.last().is_some_and(|item| !item.is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("### {}", day));
                last_msg_day = Some(day);
            }
            lines.push(text.to_string());
            continue;
        }

        last_msg_day = None;
        lines.push(format_memory_timeline_record(record));
    }
    lines.join("\n")
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

    let workspace_info = {
        let guard = session.lock().await;
        guard.workspace_info.clone()
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
) -> String {
    if token_limit == 0 {
        return String::new();
    }
    let Some(memory) = memory else {
        return String::new();
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
            return String::new();
        }
    };
    let raw_text = AgentMemory::render_memory_items(&items);
    fit_text_with_token_limit(raw_text, token_limit, tokenizer)
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

    let (session_id, title, summary) = {
        let guard = session.lock().await;
        (
            guard.session_id.clone(),
            guard.title.clone(),
            guard.summary.clone(),
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

fn resolve_worklog_db_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    if let Some(local_workspace_path) =
        resolve_default_local_workspace_root(local_workspace_id, workspace_info, session_cwd)
            .filter(|path| path.is_dir())
    {
        let worklog_db_path = local_workspace_path.join(LOCAL_WORKSPACE_WORKLOG_DB_REL_PATH);
        if worklog_db_path.is_file() {
            return Some(worklog_db_path);
        }
    }

    resolve_agent_env_root(workspace_info, session_cwd)
        .map(|root| root.join(WORKSHOP_WORKLOG_DB_REL_PATH))
        .filter(|path| path.is_file())
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

    let (session_id, session_root_dir) = if let Some(session) = input.session.as_ref() {
        let guard = session.lock().await;
        (
            normalize_optional_text(Some(guard.session_id.as_str()))
                .or_else(|| normalize_optional_text(Some(input.session_id.as_str()))),
            guard.session_root_dir.clone(),
        )
    } else {
        (
            normalize_optional_text(Some(input.session_id.as_str())),
            PathBuf::new(),
        )
    };
    let Some(session_id) = session_id else {
        return vec![];
    };

    let Some(record_file) =
        resolve_session_msg_record_path(session_id.as_str(), &session_root_dir).await
    else {
        debug!(
            "prompt.load_history_messages no record file: session_id={} session_root_dir={}",
            session_id,
            session_root_dir.display()
        );
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

        let mut text = render_history_msg_line(&msg_record, Some(input.trace.agent_name.as_str()));
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
    session_root_dir: &Path,
) -> Option<PathBuf> {
    let mut canonical_candidates = Vec::<PathBuf>::new();
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }

    if let Some(path) = non_empty_path(session_root_dir) {
        for file_name in SESSION_MSG_RECORD_FILES {
            push_unique_pathbuf(
                &mut canonical_candidates,
                path.join(session_id).join(file_name),
            );
        }
    }

    if session_id.contains('/') || session_id.contains('\\') {
        let session_path = PathBuf::from(session_id);
        if session_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("jsonl"))
            .unwrap_or(false)
        {
            push_unique_pathbuf(&mut canonical_candidates, session_path);
        } else {
            for file_name in SESSION_MSG_RECORD_FILES {
                push_unique_pathbuf(&mut canonical_candidates, session_path.join(file_name));
            }
        }
    }

    for candidate in canonical_candidates {
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

fn render_history_msg_line(record: &MsgRecordWithObject, agent_did: Option<&str>) -> String {
    let from_name = record
        .record
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            record
                .msg
                .as_ref()
                .map(|msg| msg.from.to_raw_host_name())
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| record.record.from.to_raw_host_name());
    let from_did = record
        .msg
        .as_ref()
        .map(|msg| msg.from.clone())
        .unwrap_or_else(|| record.record.from.clone());
    let update_time = resolve_history_msg_update_time(record);
    let sender = render_history_sender(from_name.as_str(), &from_did, record, agent_did);
    let timestamp = format_history_time(update_time);
    let content = record
        .msg
        .as_ref()
        .map(|msg| msg.content.content.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(normalize_history_multiline_text)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| {
            format!(
                "msg_id={} state={:?}",
                record.record.msg_id, record.record.state
            )
        });
    if content.contains('\n') {
        let indented = indent_history_message_content(content.as_str());
        format!("- {} {}:\n{}", timestamp, sender, indented)
    } else {
        format!("- {} {}: {}", timestamp, sender, content)
    }
}

fn format_history_date(timestamp_ms: u64) -> String {
    let dt = history_datetime_utc(timestamp_ms);
    dt.format("%Y-%m-%d").to_string()
}

fn format_history_time(timestamp_ms: u64) -> String {
    let dt = history_datetime_utc(timestamp_ms);
    dt.format("%H:%M").to_string()
}

fn history_datetime_utc(timestamp_ms: u64) -> DateTime<Utc> {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now)
}

fn render_history_sender(
    from_name: &str,
    from_did: &DID,
    record: &MsgRecordWithObject,
    agent_did: Option<&str>,
) -> String {
    let from_did = from_did.to_string();
    let agent_did = agent_did.unwrap_or_default().trim();
    let from_short = compact_history_name(from_name);
    let is_me = record.record.box_kind == BoxKind::Outbox
        || (!agent_did.is_empty() && from_did.eq_ignore_ascii_case(agent_did));
    if is_me {
        let mut agent_name = from_short.clone();
        if agent_name.is_empty() {
            agent_name = compact_history_name(extract_name_from_did(agent_did).as_str());
        }
        if agent_name.is_empty() {
            agent_name = "Agent".to_string();
        }
        return format!("{}(me)", capitalize_ascii(agent_name.as_str()));
    }

    if from_did.starts_with("did:bns:") {
        let name = if from_short.is_empty() {
            "unknown".to_string()
        } else {
            from_short
        };
        return format!("{}({})", name, from_did);
    }
    if from_short.is_empty() {
        from_did
    } else {
        from_short
    }
}

fn extract_name_from_did(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() {
        return String::new();
    }
    let core = raw.strip_prefix("did:").unwrap_or(raw);
    let mut parts = core.splitn(2, ':');
    let _method = parts.next();
    let suffix = parts.next().unwrap_or(core);
    suffix.to_string()
}

fn compact_history_name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    value
        .split('.')
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(value)
        .to_string()
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    out.push_str(chars.as_str());
    out
}

fn indent_history_message_content(value: &str) -> String {
    value
        .lines()
        .map(|line| format!("  {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_history_multiline_text(input: &str) -> String {
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

fn sanitize_worklog_digest(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return String::new();
    }
    let chars = compact.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return compact;
    }
    let mut out = chars.into_iter().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn parse_step_index_limit_from_text(value: &str) -> Option<(u32, u32)> {
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] != b"step" {
            i += 1;
            continue;
        }
        let mut j = i + 4;
        while j < bytes.len() && !bytes[j].is_ascii_digit() {
            j += 1;
        }
        let start_step = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if start_step == j {
            i += 1;
            continue;
        }
        let step_index = lower[start_step..j].parse::<u32>().ok();
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'/' {
            i += 1;
            continue;
        }
        j += 1;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        let start_limit = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if start_limit == j {
            i += 1;
            continue;
        }
        let step_limit = lower[start_limit..j].parse::<u32>().ok();
        if let (Some(step_index), Some(step_limit)) = (step_index, step_limit) {
            return Some((step_index, step_limit));
        }
        i += 1;
    }
    None
}

fn format_step_header_line(
    timestamp_ms: u64,
    behavior: Option<&str>,
    step_index: Option<u32>,
    success_count: u32,
    failed_count: u32,
) -> String {
    let behavior = behavior
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");
    let step_index_text = step_index
        .map(|value| value.to_string())
        .unwrap_or("-".to_string());

    format!(
        "#### {} Behavior {} Step {} : SUCCESS ({}), FAILED ({})",
        format_history_time(timestamp_ms),
        behavior,
        step_index_text,
        success_count,
        failed_count
    )
}

fn count_success_failed_db(records: &[WorklogRecord]) -> (u32, u32) {
    let mut success = 0_u32;
    let mut failed = 0_u32;
    for record in records {
        if record.status.trim().eq_ignore_ascii_case("OK") {
            success = success.saturating_add(1);
        } else if record.status.trim().eq_ignore_ascii_case("FAILED") {
            failed = failed.saturating_add(1);
        }
    }
    (success, failed)
}

fn count_success_failed_runtime(records: &[RuntimeWorklogRecord]) -> (u32, u32) {
    let mut success = 0_u32;
    let mut failed = 0_u32;
    for record in records {
        if record.status.trim().eq_ignore_ascii_case("OK") {
            success = success.saturating_add(1);
        } else if record.status.trim().eq_ignore_ascii_case("FAILED") {
            failed = failed.saturating_add(1);
        }
    }
    (success, failed)
}

fn render_worklog_message_sender(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return "unknown".to_string();
    }
    if value.starts_with("did:bns:") {
        let name = compact_history_name(extract_name_from_did(value).as_str());
        if name.is_empty() {
            return value.to_string();
        }
        return format!("{}({})", name, value);
    }
    if value.starts_with("did:") {
        let name = compact_history_name(extract_name_from_did(value).as_str());
        if !name.is_empty() {
            return name;
        }
        return value.to_string();
    }
    let name = compact_history_name(value);
    if name.is_empty() {
        value.to_string()
    } else {
        name
    }
}

fn sanitize_worklog_message_text(value: &str, max_chars: usize) -> String {
    let sanitized = value
        .replace("```", "'''")
        .replace("<</WorkspaceWorklog:OBSERVATION>>", "")
        .replace("<<WorkspaceWorklog:OBSERVATION>>", "");
    let normalized = normalize_history_multiline_text(sanitized.as_str());
    if normalized.is_empty() {
        return String::new();
    }
    let chars = normalized.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return normalized;
    }
    let mut out = chars.into_iter().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn render_worklog_get_message_line(timestamp: u64, payload: &Json) -> String {
    let from = payload
        .get("from")
        .and_then(Json::as_str)
        .unwrap_or("Unknown");
    let sender = render_worklog_message_sender(from);
    let content = payload
        .get("content_digest")
        .or_else(|| payload.get("snippet"))
        .or_else(|| payload.get("content"))
        .or_else(|| payload.get("message"))
        .and_then(Json::as_str)
        .map(|value| sanitize_worklog_message_text(value, 220))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_string());
    if content.contains('\n') {
        let indented = indent_history_message_content(content.as_str());
        format!(
            "- {} {}:\n{}",
            format_history_time(timestamp),
            sender,
            indented
        )
    } else {
        format!(
            "- {} {}: {}",
            format_history_time(timestamp),
            sender,
            content
        )
    }
}

fn render_worklog_status_text(status: &str, reason_digest: Option<&str>) -> String {
    if status.trim().eq_ignore_ascii_case("OK") {
        return "OK".to_string();
    }
    reason_digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| sanitize_worklog_digest(value, 80))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| status.trim().to_string())
}

fn extract_worklog_target(payload: Option<&serde_json::Map<String, Json>>) -> Option<String> {
    let payload = payload?;
    for key in ["path", "file_path", "file", "target"] {
        let Some(value) = payload.get(key).and_then(Json::as_str) else {
            continue;
        };
        let value = sanitize_worklog_digest(value, 120);
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn is_tool_call_action_payload(payload: &Json) -> bool {
    payload
        .get("action_type")
        .and_then(Json::as_str)
        .map(str::trim)
        .map(|value| value.eq_ignore_ascii_case("tool_call"))
        .unwrap_or(false)
}

fn render_nested_db_worklog_line(record: &WorklogRecord) -> Option<String> {
    let status_text = render_worklog_status_text(
        record.status.as_str(),
        record
            .error
            .as_ref()
            .and_then(|error| error.reason_digest.as_deref()),
    );
    match record.record_type {
        WorklogRecordType::ReplyMessage => {
            let said = record
                .payload
                .get("content_digest")
                .or_else(|| record.payload.get("said"))
                .and_then(Json::as_str)
                .map(|value| sanitize_worklog_digest(value, 140))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_string());
            Some(format!("Reply: \"{}\"", said))
        }
        WorklogRecordType::FunctionRecord => {
            let tool_name = record
                .payload
                .get("tool_name")
                .and_then(Json::as_str)
                .unwrap_or("function_call");
            let target = extract_worklog_target(record.payload.as_object());
            Some(format!(
                "{}{} -> {}",
                tool_name,
                target
                    .map(|value| format!(" {}", value))
                    .unwrap_or_default(),
                status_text
            ))
        }
        WorklogRecordType::ActionRecord => {
            if is_tool_call_action_payload(&record.payload) {
                return None;
            }
            let action_type = record
                .payload
                .get("action_type")
                .and_then(Json::as_str)
                .unwrap_or("action");
            let target = record
                .payload
                .get("cmd_digest")
                .or_else(|| record.payload.get("command"))
                .and_then(Json::as_str)
                .map(|value| sanitize_worklog_digest(value, 120))
                .filter(|value| !value.is_empty())
                .or_else(|| extract_worklog_target(record.payload.as_object()));
            let result_digest = record
                .payload
                .get("result_digest")
                .and_then(Json::as_str)
                .map(|value| sanitize_worklog_digest(value, 180))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| status_text.clone());
            if action_type.eq_ignore_ascii_case("bash") {
                let cmd = target.unwrap_or_else(|| "bash".to_string());
                return Some(format!("Run {} => {}", cmd, result_digest));
            }
            Some(format!(
                "{}{} -> {}",
                action_type,
                target
                    .map(|value| format!(" {}", value))
                    .unwrap_or_default(),
                result_digest
            ))
        }
        WorklogRecordType::CreateSubAgent => {
            let name = record
                .payload
                .get("subagent_name")
                .and_then(Json::as_str)
                .or_else(|| record.payload.get("subagent_did").and_then(Json::as_str))
                .unwrap_or("-");
            Some(format!(
                "create_sub_agent {} -> {}",
                sanitize_worklog_digest(name, 80),
                status_text
            ))
        }
        _ => None,
    }
}

fn render_db_worklog_top_level_line(record: &WorklogRecord) -> Option<String> {
    if let Some(nested) = render_nested_db_worklog_line(record) {
        return Some(format!(
            "- {} {}",
            format_history_time(record.timestamp),
            nested
        ));
    }
    let legacy = render_worklog_prompt_line(record);
    let legacy = legacy.trim();
    if legacy.is_empty() {
        None
    } else {
        Some(format!(
            "- {} {}",
            format_history_time(record.timestamp),
            legacy
        ))
    }
}

fn render_db_worklog_timeline_records(
    worklog_records: &[WorklogRecord],
) -> Vec<MemoryTimelineRecord> {
    let mut timeline = worklog_records
        .iter()
        .filter(|record| !record.commit_state.eq_ignore_ascii_case("PENDING"))
        .cloned()
        .collect::<Vec<_>>();
    timeline.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.seq.cmp(&b.seq))
            .then_with(|| a.id.cmp(&b.id))
    });

    let by_id = timeline
        .iter()
        .map(|record| (record.id.clone(), record.clone()))
        .collect::<HashMap<_, _>>();
    let referenced_ids = timeline
        .iter()
        .filter(|record| record.record_type == WorklogRecordType::StepSummary)
        .flat_map(|record| {
            record
                .payload
                .get("refs")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();

    let mut output = Vec::<MemoryTimelineRecord>::new();
    let mut current_day = String::new();
    for record in &timeline {
        if record.record_type == WorklogRecordType::ActionRecord
            && is_tool_call_action_payload(&record.payload)
        {
            continue;
        }
        match record.record_type {
            WorklogRecordType::GetMessage => {
                let day = format_history_date(record.timestamp);
                if day != current_day {
                    output.push(MemoryTimelineRecord {
                        update_time: record.timestamp,
                        text: format!("### {}", day),
                        source_label: "worklog",
                        source_order: 30,
                    });
                    current_day = day;
                }
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: render_worklog_get_message_line(record.timestamp, &record.payload),
                    source_label: "worklog",
                    source_order: 30,
                });
            }
            WorklogRecordType::StepSummary => {
                let day = format_history_date(record.timestamp);
                if day != current_day {
                    output.push(MemoryTimelineRecord {
                        update_time: record.timestamp,
                        text: format!("### {}", day),
                        source_label: "worklog",
                        source_order: 30,
                    });
                    current_day = day;
                }
                let mut nested_records = record
                    .payload
                    .get("refs")
                    .and_then(Json::as_array)
                    .map(|refs| {
                        refs.iter()
                            .filter_map(Json::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .filter_map(|id| by_id.get(id).cloned())
                            .filter(|item| item.record_type != WorklogRecordType::StepSummary)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                nested_records.sort_by(|a, b| {
                    a.timestamp
                        .cmp(&b.timestamp)
                        .then_with(|| a.seq.cmp(&b.seq))
                        .then_with(|| a.id.cmp(&b.id))
                });
                let nested_lines = nested_records
                    .iter()
                    .filter_map(render_nested_db_worklog_line)
                    .collect::<Vec<_>>();

                let (success_count, failed_count) = count_success_failed_db(&nested_records);
                let parsed_index_limit = record
                    .summary
                    .as_deref()
                    .and_then(parse_step_index_limit_from_text)
                    .or_else(|| {
                        record
                            .payload
                            .get("result_digest")
                            .and_then(Json::as_str)
                            .and_then(parse_step_index_limit_from_text)
                    })
                    .or_else(|| {
                        record
                            .payload
                            .get("did_digest")
                            .and_then(Json::as_str)
                            .and_then(parse_step_index_limit_from_text)
                    });
                let step_index = record
                    .step_index
                    .or_else(|| {
                        record
                            .step_id
                            .as_deref()
                            .and_then(parse_step_index_from_step_id)
                    })
                    .or_else(|| parsed_index_limit.map(|value| value.0));
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: format_step_header_line(
                        record.timestamp,
                        record.behavior.as_deref(),
                        step_index,
                        success_count,
                        failed_count,
                    ),
                    source_label: "worklog",
                    source_order: 30,
                });

                for nested in nested_lines {
                    output.push(MemoryTimelineRecord {
                        update_time: record.timestamp,
                        text: format!("- {}", nested),
                        source_label: "worklog",
                        source_order: 30,
                    });
                }
            }
            _ => {
                if referenced_ids.contains(record.id.as_str()) {
                    continue;
                }
                if let Some(line) = render_db_worklog_top_level_line(record) {
                    let day = format_history_date(record.timestamp);
                    if day != current_day {
                        output.push(MemoryTimelineRecord {
                            update_time: record.timestamp,
                            text: format!("### {}", day),
                            source_label: "worklog",
                            source_order: 30,
                        });
                        current_day = day;
                    }
                    output.push(MemoryTimelineRecord {
                        update_time: record.timestamp,
                        text: line,
                        source_label: "worklog",
                        source_order: 30,
                    });
                }
            }
        }
    }
    output
}

#[derive(Clone, Debug)]
struct RuntimeWorklogRecord {
    id: String,
    timestamp: u64,
    record_type: String,
    status: String,
    behavior: Option<String>,
    step_index: Option<u32>,
    payload: Json,
    summary: Option<String>,
    prompt_digest: Option<String>,
    error_reason: Option<String>,
}

fn runtime_worklog_is_type(record_type: &str, expected: &str) -> bool {
    record_type.trim().eq_ignore_ascii_case(expected)
}

fn render_nested_runtime_worklog_line(record: &RuntimeWorklogRecord) -> Option<String> {
    let status_text =
        render_worklog_status_text(record.status.as_str(), record.error_reason.as_deref());
    if runtime_worklog_is_type(record.record_type.as_str(), "ReplyMessage") {
        let said = record
            .payload
            .get("content_digest")
            .or_else(|| record.payload.get("said"))
            .and_then(Json::as_str)
            .map(|value| sanitize_worklog_digest(value, 140))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "-".to_string());
        return Some(format!("Reply: \"{}\"", said));
    }
    if runtime_worklog_is_type(record.record_type.as_str(), "FunctionRecord") {
        let tool_name = record
            .payload
            .get("tool_name")
            .and_then(Json::as_str)
            .unwrap_or("function_call");
        let target = extract_worklog_target(record.payload.as_object());
        return Some(format!(
            "{}{} -> {}",
            tool_name,
            target
                .map(|value| format!(" {}", value))
                .unwrap_or_default(),
            status_text
        ));
    }
    if runtime_worklog_is_type(record.record_type.as_str(), "ActionRecord") {
        if is_tool_call_action_payload(&record.payload) {
            return None;
        }
        let action_type = record
            .payload
            .get("action_type")
            .and_then(Json::as_str)
            .unwrap_or("action");
        let target = record
            .payload
            .get("cmd_digest")
            .or_else(|| record.payload.get("command"))
            .and_then(Json::as_str)
            .map(|value| sanitize_worklog_digest(value, 120))
            .filter(|value| !value.is_empty())
            .or_else(|| extract_worklog_target(record.payload.as_object()));
        let result_digest = record
            .payload
            .get("result_digest")
            .and_then(Json::as_str)
            .map(|value| sanitize_worklog_digest(value, 180))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| status_text.clone());
        if action_type.eq_ignore_ascii_case("bash") {
            let cmd = target.unwrap_or_else(|| "bash".to_string());
            return Some(format!("Run {} => {}", cmd, result_digest));
        }
        return Some(format!(
            "{}{} -> {}",
            action_type,
            target
                .map(|value| format!(" {}", value))
                .unwrap_or_default(),
            result_digest
        ));
    }
    if runtime_worklog_is_type(record.record_type.as_str(), "CreateSubAgent") {
        let name = record
            .payload
            .get("subagent_name")
            .and_then(Json::as_str)
            .or_else(|| record.payload.get("subagent_did").and_then(Json::as_str))
            .unwrap_or("-");
        return Some(format!(
            "create_sub_agent {} -> {}",
            sanitize_worklog_digest(name, 80),
            status_text
        ));
    }
    None
}

fn render_runtime_worklog_legacy_line(record: &RuntimeWorklogRecord) -> Option<String> {
    let digest = record
        .prompt_digest
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| sanitize_worklog_digest(value, 200))
        .filter(|value| !value.is_empty())
        .or_else(|| {
            record
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| sanitize_worklog_digest(value, 200))
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            let payload = sanitize_json_compact(&record.payload);
            let compact = sanitize_worklog_digest(payload.as_str(), 200);
            if compact.is_empty() {
                None
            } else {
                Some(compact)
            }
        });

    let line = if let Some(digest) = digest {
        format!(
            "{} status={} {}",
            record.record_type.trim(),
            record.status.trim(),
            digest
        )
    } else {
        format!(
            "{} status={}",
            record.record_type.trim(),
            record.status.trim()
        )
    };
    let line = line.trim();
    if line.is_empty() {
        None
    } else {
        Some(format!(
            "- {} {}",
            format_history_time(record.timestamp),
            line
        ))
    }
}

fn render_runtime_worklog_top_level_line(record: &RuntimeWorklogRecord) -> Option<String> {
    if let Some(nested) = render_nested_runtime_worklog_line(record) {
        return Some(format!(
            "- {} {}",
            format_history_time(record.timestamp),
            nested
        ));
    }
    render_runtime_worklog_legacy_line(record)
}

fn parse_timestamp_millis_text(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = value.parse::<u64>() {
        return Some(parsed);
    }
    if let Ok(parsed) = value.parse::<i64>() {
        if parsed > 0 {
            return Some(parsed as u64);
        }
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp_millis().max(0) as u64)
}

fn parse_timestamp_millis(value: &Json) -> Option<u64> {
    if let Some(parsed) = value.as_u64() {
        return Some(parsed);
    }
    if let Some(parsed) = value.as_i64() {
        if parsed > 0 {
            return Some(parsed as u64);
        }
    }
    value.as_str().and_then(parse_timestamp_millis_text)
}

fn resolve_session_runtime_worklog_update_time(item: &Json, fallback: u64) -> u64 {
    for key in ["timestamp", "updated_at_ms", "created_at_ms", "ts"] {
        let Some(raw) = item.get(key) else {
            continue;
        };
        if let Some(parsed) = parse_timestamp_millis(raw) {
            return parsed;
        }
    }
    fallback
}

fn parse_step_index_from_step_id(step_id: &str) -> Option<u32> {
    let digits = step_id
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn dedup_last_step_index(current_step_index: u32) -> Option<u32> {
    current_step_index.checked_sub(1)
}

fn extract_runtime_worklog_step_index(item: &Json) -> Option<u32> {
    item.get("step_index")
        .and_then(Json::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| {
            item.get("step_index")
                .and_then(Json::as_i64)
                .and_then(|value| {
                    if value < 0 {
                        None
                    } else {
                        u32::try_from(value as u64).ok()
                    }
                })
        })
        .or_else(|| {
            item.get("step_id")
                .and_then(Json::as_str)
                .and_then(parse_step_index_from_step_id)
        })
}

fn is_pending_session_runtime_worklog(item: &Json) -> bool {
    item.get("commit_state")
        .and_then(Json::as_str)
        .map(str::trim)
        .map(|state| state.eq_ignore_ascii_case("PENDING"))
        .unwrap_or(false)
}

fn parse_runtime_worklog_record(
    item: Json,
    index: usize,
    fallback_update_time: u64,
) -> RuntimeWorklogRecord {
    let step_index = extract_runtime_worklog_step_index(&item);
    let id = item
        .get("id")
        .or_else(|| item.get("log_id"))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("runtime-{index}"));
    let record_type = item
        .get("type")
        .or_else(|| item.get("log_type"))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Worklog".to_string());
    let status = item
        .get("status")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let behavior = item
        .get("behavior")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let payload = item.get("payload").cloned().unwrap_or(Json::Null);
    let summary = item
        .get("summary")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let prompt_digest = item
        .get("prompt_view")
        .and_then(Json::as_object)
        .and_then(|view| view.get("digest"))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let error_reason = item
        .get("error")
        .and_then(Json::as_object)
        .and_then(|error| error.get("reason_digest"))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    RuntimeWorklogRecord {
        id,
        timestamp: resolve_session_runtime_worklog_update_time(&item, fallback_update_time),
        record_type,
        status,
        behavior,
        step_index,
        payload,
        summary,
        prompt_digest,
        error_reason,
    }
}

fn render_runtime_worklog_timeline_records(
    runtime_records: &[RuntimeWorklogRecord],
) -> Vec<MemoryTimelineRecord> {
    let mut timeline = runtime_records.to_vec();
    timeline.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));

    let by_id = timeline
        .iter()
        .map(|record| (record.id.clone(), record.clone()))
        .collect::<HashMap<_, _>>();
    let referenced_ids = timeline
        .iter()
        .filter(|record| runtime_worklog_is_type(record.record_type.as_str(), "StepSummary"))
        .flat_map(|record| {
            record
                .payload
                .get("refs")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();

    let mut output = Vec::<MemoryTimelineRecord>::new();
    let mut current_day = String::new();
    for record in &timeline {
        if runtime_worklog_is_type(record.record_type.as_str(), "ActionRecord")
            && is_tool_call_action_payload(&record.payload)
        {
            continue;
        }
        if runtime_worklog_is_type(record.record_type.as_str(), "GetMessage") {
            let day = format_history_date(record.timestamp);
            if day != current_day {
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: format!("### {}", day),
                    source_label: "worklog",
                    source_order: 30,
                });
                current_day = day;
            }
            output.push(MemoryTimelineRecord {
                update_time: record.timestamp,
                text: render_worklog_get_message_line(record.timestamp, &record.payload),
                source_label: "worklog",
                source_order: 30,
            });
            continue;
        }
        if runtime_worklog_is_type(record.record_type.as_str(), "StepSummary") {
            let day = format_history_date(record.timestamp);
            if day != current_day {
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: format!("### {}", day),
                    source_label: "worklog",
                    source_order: 30,
                });
                current_day = day;
            }
            let mut nested_records = record
                .payload
                .get("refs")
                .and_then(Json::as_array)
                .map(|refs| {
                    refs.iter()
                        .filter_map(Json::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .filter_map(|id| by_id.get(id).cloned())
                        .filter(|item| {
                            !runtime_worklog_is_type(item.record_type.as_str(), "StepSummary")
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            nested_records
                .sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));
            let nested_lines = nested_records
                .iter()
                .filter_map(render_nested_runtime_worklog_line)
                .collect::<Vec<_>>();
            let (success_count, failed_count) = count_success_failed_runtime(&nested_records);
            let parsed_index_limit = record
                .summary
                .as_deref()
                .and_then(parse_step_index_limit_from_text)
                .or_else(|| {
                    record
                        .payload
                        .get("result_digest")
                        .and_then(Json::as_str)
                        .and_then(parse_step_index_limit_from_text)
                })
                .or_else(|| {
                    record
                        .payload
                        .get("did_digest")
                        .and_then(Json::as_str)
                        .and_then(parse_step_index_limit_from_text)
                });
            let step_index = record
                .step_index
                .or_else(|| parsed_index_limit.map(|value| value.0));
            output.push(MemoryTimelineRecord {
                update_time: record.timestamp,
                text: format_step_header_line(
                    record.timestamp,
                    record.behavior.as_deref(),
                    step_index,
                    success_count,
                    failed_count,
                ),
                source_label: "worklog",
                source_order: 30,
            });
            for nested in nested_lines {
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: format!("- {}", nested),
                    source_label: "worklog",
                    source_order: 30,
                });
            }
            continue;
        }

        if referenced_ids.contains(record.id.as_str()) {
            continue;
        }
        if let Some(line) = render_runtime_worklog_top_level_line(record) {
            let day = format_history_date(record.timestamp);
            if day != current_day {
                output.push(MemoryTimelineRecord {
                    update_time: record.timestamp,
                    text: format!("### {}", day),
                    source_label: "worklog",
                    source_order: 30,
                });
                current_day = day;
            }
            output.push(MemoryTimelineRecord {
                update_time: record.timestamp,
                text: line,
                source_label: "worklog",
                source_order: 30,
            });
        }
    }
    output
}

fn fit_worklog_timeline_records_with_limit(
    candidate_records: Vec<MemoryTimelineRecord>,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }

    let mut records = Vec::<MemoryTimelineRecord>::new();
    let mut used_tokens = 0_u32;
    for mut item in candidate_records {
        let mut text = item.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        text = fit_text_with_token_limit(text, token_limit.min(128).max(24), tokenizer);
        if text.is_empty() {
            continue;
        }
        item.text = text;

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

async fn load_session_runtime_worklog_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }
    let Some(session) = input.session.as_ref() else {
        return vec![];
    };

    let session_worklog = {
        let guard = session.lock().await;
        guard.worklog.clone()
    };
    if session_worklog.is_empty() {
        return vec![];
    }

    let base_update_time = Utc::now().timestamp_millis().max(0) as u64;
    let total = session_worklog.len();
    let exclude_step_index = dedup_last_step_index(input.trace.step_idx);
    let mut runtime_records = Vec::<RuntimeWorklogRecord>::new();
    for (index, item) in session_worklog.into_iter().enumerate() {
        if is_pending_session_runtime_worklog(&item) {
            continue;
        }
        if exclude_step_index.is_some()
            && extract_runtime_worklog_step_index(&item) == exclude_step_index
        {
            continue;
        }
        let fallback_update_time = base_update_time.saturating_sub((total - index) as u64);
        runtime_records.push(parse_runtime_worklog_record(
            item,
            index,
            fallback_update_time,
        ));
    }

    let candidate_records = render_runtime_worklog_timeline_records(&runtime_records);
    fit_worklog_timeline_records_with_limit(candidate_records, token_limit, tokenizer)
}

async fn load_workspace_worklog_with_limit(
    input: &BehaviorExecInput,
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    if token_limit == 0 {
        return vec![];
    }

    let (session_id, workspace_id, workspace_info, session_cwd, session_worklog_db_path) =
        if let Some(session) = input.session.as_ref() {
            let guard = session.lock().await;
            (
                normalize_optional_text(Some(guard.session_id.as_str()))
                    .or_else(|| normalize_optional_text(Some(input.session_id.as_str()))),
                guard.local_workspace_id.clone(),
                guard.workspace_info.clone(),
                guard.pwd.clone(),
                guard.resolve_workspace_worklog_db_path(),
            )
        } else {
            (
                normalize_optional_text(Some(input.session_id.as_str())),
                None,
                None,
                PathBuf::new(),
                None,
            )
        };

    if workspace_id.is_none() {
        return load_session_runtime_worklog_with_limit(input, token_limit, tokenizer).await;
    }

    let Some(worklog_db_path) = session_worklog_db_path.or_else(|| {
        resolve_worklog_db_path(
            workspace_id.as_deref(),
            workspace_info.as_ref(),
            &session_cwd,
        )
    }) else {
        return load_session_runtime_worklog_with_limit(input, token_limit, tokenizer).await;
    };

    let query_limit = usize::try_from(token_limit.saturating_mul(2))
        .unwrap_or(usize::MAX)
        .clamp(16, 256);
    let worklog_service =
        match WorklogService::new(WorklogToolConfig::with_db_path(worklog_db_path.clone())) {
            Ok(service) => service,
            Err(err) => {
                warn!(
                    "prompt.load_workspace_worklog create service failed: path={} err={}",
                    worklog_db_path.display(),
                    err
                );
                return load_session_runtime_worklog_with_limit(input, token_limit, tokenizer)
                    .await;
            }
        };

    let mut worklog_records = match worklog_service
        .list_worklog_records(WorklogListOptions {
            owner_session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
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
            return load_session_runtime_worklog_with_limit(input, token_limit, tokenizer).await;
        }
    };
    if worklog_records.is_empty() && workspace_id.is_some() {
        worklog_records = match worklog_service
            .list_worklog_records(WorklogListOptions {
                owner_session_id: session_id,
                workspace_id: None,
                limit: Some(query_limit),
                ..Default::default()
            })
            .await
        {
            Ok(records) => records,
            Err(err) => {
                warn!(
                    "prompt.load_workspace_worklog fallback list(session-only) failed: path={} err={}",
                    worklog_db_path.display(),
                    err
                );
                vec![]
            }
        };
    }
    if let Some(exclude_step_index) = dedup_last_step_index(input.trace.step_idx) {
        worklog_records.retain(|record| {
            let record_step_index = record.step_index.or_else(|| {
                record
                    .step_id
                    .as_deref()
                    .and_then(parse_step_index_from_step_id)
            });
            record_step_index != Some(exclude_step_index)
        });
    }
    if worklog_records.is_empty() {
        return load_session_runtime_worklog_with_limit(input, token_limit, tokenizer).await;
    }

    let candidate_records = render_db_worklog_timeline_records(&worklog_records);
    let records =
        fit_worklog_timeline_records_with_limit(candidate_records, token_limit, tokenizer);
    return records;
}

async fn collect_workspace_skill_roots(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Vec<PathBuf> {
    let mut skill_roots = Vec::<PathBuf>::new();
    if let Some(local_workspace_path) =
        resolve_default_local_workspace_root(local_workspace_id, workspace_info, session_cwd)
            .filter(|path| path.is_dir())
    {
        push_unique_pathbuf(
            &mut skill_roots,
            local_workspace_path.join(LOCAL_WORKSPACE_SKILLS_DIR),
        );
    }
    if let Some(workspace_root) = resolve_session_workspace_root(workspace_info, session_cwd) {
        push_unique_pathbuf(
            &mut skill_roots,
            workspace_root.join(LOCAL_WORKSPACE_SKILLS_DIR),
        );
    }
    if let Some(agent_env_root) = resolve_agent_env_root(workspace_info, session_cwd) {
        push_unique_pathbuf(
            &mut skill_roots,
            agent_env_root.join(LOCAL_WORKSPACE_SKILLS_DIR),
        );
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
    use std::sync::Arc;

    use super::*;
    use crate::agent_session::AgentSession;
    use crate::agent_tool::AgentTool;
    use crate::agent_tool::{AgentMemory, AgentMemoryConfig};
    use crate::behavior::types::{SessionRuntimeContext, StepLimits};
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
    async fn build_produces_valid_prompt() {
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 2,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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

        let req = PromptBuilder::build(&input, &input.behavior_cfg, &MockTokenizer, None, None)
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
        assert!(!system.contains("<<skills>>"));
        assert!(system.contains("You are a helpful assistant."));
        assert!(system.contains("Process rules."));
    }

    #[tokio::test]
    async fn build_memory_prompt_keeps_head_and_tail_when_budget_is_small() {
        let temp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(temp.path()))
            .await
            .expect("create agent memory");
        memory
            .set_memory(
                "/project/context",
                r#"{"type":"fact","summary":"DYNAMIC_MEMORY_SHOULD_NOT_APPEAR","importance":8,"tags":["project"]}"#,
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
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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

        let mut cfg = BehaviorConfig::default();
        cfg.memory.total_limit = 8;
        cfg.memory.first_prompt = Some("HEAD_FIXED alpha beta gamma".to_string());
        cfg.memory.last_prompt = Some("TAIL_FIXED one two three".to_string());
        cfg.memory.agent_memory = BehaviorMemoryBucketConfig {
            limit: 256,
            max_percent: Some(1.0),
            is_enable: true,
        };

        let tokenizer = MockTokenizer;
        let prompt = build_memory_prompt_text(
            &input,
            &cfg,
            vec!["project".to_string()],
            cfg.memory.total_limit,
            &tokenizer,
            Some(memory),
        )
        .await;

        assert!(prompt.contains("HEAD_FIXED alpha beta gamma"));
        assert!(prompt.contains("TAIL_FIXED one two three"));
        assert!(!prompt.contains("DYNAMIC_MEMORY_SHOULD_NOT_APPEAR"));
        println!("\n[memory_text prompt preview]\n{prompt}\n");
        assert!(tokenizer.count_tokens(prompt.as_str()) > cfg.memory.total_limit);
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
                r#"{"type":"preference","summary":"用户偏好简洁回复","importance":7,"tags":["style"]}"#,
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
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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

        let memory_text = load_agent_memory_with_limit(
            &input,
            &["style".to_string()],
            200,
            &MockTokenizer,
            Some(memory),
        )
        .await;

        assert!(
            !memory_text.trim().is_empty(),
            "expected remembered things content"
        );
        assert!(memory_text.contains("user/preference/style"));
        assert!(memory_text.contains("用户偏好简洁回复"));
        println!("\n[memory_text prompt preview]\n{memory_text}\n");
    }

    #[tokio::test]
    async fn load_history_messages_with_limit_reads_session_jsonl_reverse() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let session_id = "session-1";
        let session_dir = agent_env_root.join("sessions").join(session_id);
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
        session.pwd = agent_env_root.clone();
        session.session_root_dir = agent_env_root.join("sessions");
        let input = BehaviorExecInput {
            session_id: session_id.to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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
    async fn build_memory_prompt_text_includes_history_messages_timeline() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let session_id = "session-1";
        let session_dir = agent_env_root.join("sessions").join(session_id);
        tokio::fs::create_dir_all(&session_dir)
            .await
            .expect("create session dir");

        let line1 = json!({
            "record": {
                "record_id": "r1",
                "box_kind": "INBOX",
                "msg_id": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "to": "did:web:bob.example.com",
                "created_at_ms": 1772578920000_u64,
                "updated_at_ms": 1772578920000_u64,
                "sort_key": 1772578920000_u64,
                "tags": []
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:bob.example.com"],
                "kind": "chat",
                "created_at_ms": 1772578920000_u64,
                "content": {
                    "content": "i want to buy a new laptop"
                }
            }
        });
        let line2 = json!({
            "record": {
                "record_id": "r2",
                "box_kind": "INBOX",
                "msg_id": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "to": "did:web:bob.example.com",
                "created_at_ms": 1772578980000_u64,
                "updated_at_ms": 1772578980000_u64,
                "sort_key": 1772578980000_u64,
                "tags": []
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:bob.example.com"],
                "kind": "chat",
                "created_at_ms": 1772578980000_u64,
                "content": {
                    "content": "can you help me?"
                }
            }
        });
        let line3 = json!({
            "record": {
                "record_id": "r3",
                "box_kind": "OUTBOX",
                "msg_id": "sha256:cccccccccccccccccccccccccccccccc",
                "msg_kind": "chat",
                "state": "SENT",
                "from": "did:web:jarvis.example.com",
                "to": "did:web:alice.example.com",
                "created_at_ms": 1772579040000_u64,
                "updated_at_ms": 1772579040000_u64,
                "sort_key": 1772579040000_u64,
                "tags": []
            },
            "msg": {
                "from": "did:web:jarvis.example.com",
                "to": ["did:web:alice.example.com"],
                "kind": "chat",
                "created_at_ms": 1772579040000_u64,
                "content": {
                    "content": "history line3 content\nlong content\nlong content"
                }
            }
        });
        let line4 = json!({
            "record": {
                "record_id": "r4",
                "box_kind": "INBOX",
                "msg_id": "sha256:dddddddddddddddddddddddddddddddd",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:bns:bob",
                "to": "did:web:jarvis.example.com",
                "created_at_ms": 1772582520000_u64,
                "updated_at_ms": 1772582520000_u64,
                "sort_key": 1772582520000_u64,
                "tags": []
            },
            "msg": {
                "from": "did:bns:bob",
                "to": ["did:web:jarvis.example.com"],
                "kind": "chat",
                "created_at_ms": 1772582520000_u64,
                "content": {
                    "content": "hello, how are you?"
                }
            }
        });

        tokio::fs::write(
            session_dir.join("msg_record.jsonl"),
            format!("{line1}\n{line2}\n{line3}\n{line4}\n"),
        )
        .await
        .expect("write msg record file");

        let mut session = AgentSession::new(session_id, "did:web:agent.example.com", None);
        session.pwd = agent_env_root.clone();
        session.session_root_dir = agent_env_root.join("sessions");
        let input = BehaviorExecInput {
            session_id: session_id.to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-history-prompt".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-history".to_string(),
                session_id: "session-test".to_string(),
            },
            input_prompt: "print history_messages".to_string(),
            last_step_prompt: String::new(),
            role_md: "You are a helpful assistant.".to_string(),
            self_md: "Self description.".to_string(),
            behavior_prompt: "rules".to_string(),
            limits: StepLimits::default(),
            behavior_cfg: BehaviorConfig::default(),
            session: Some(Arc::new(Mutex::new(session))),
        };

        let mut cfg = BehaviorConfig::default();
        cfg.memory.total_limit = 256;
        cfg.memory.history_messages = BehaviorMemoryBucketConfig {
            limit: 256,
            max_percent: Some(1.0),
            is_enable: true,
        };

        let prompt = build_memory_prompt_text(
            &input,
            &cfg,
            vec![],
            cfg.memory.total_limit,
            &MockTokenizer,
            None,
        )
        .await;

        println!("\n[history_messages prompt preview]\n{prompt}\n");
        assert!(prompt.contains("<<memory>>"));
        assert!(prompt.contains("## Timeline"));
        assert!(prompt.contains("### 2026-03-03"));
        assert!(prompt.contains("- 23:02 alice: i want to buy a new laptop"));
        assert!(prompt.contains("- 23:03 alice: can you help me?"));
        assert!(prompt.contains("- 23:04 Jarvis(me):"));
        assert!(prompt.contains("  history line3 content"));
        assert!(prompt.contains("  long content"));
        assert!(prompt.contains("### 2026-03-04"));
        assert!(prompt.contains("- 00:02 bob(did:bns:bob): hello, how are you?"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_reads_from_worklog_db() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let local_workspace_id = "ws-demo";
        let local_workspace_path = agent_env_root.join("workspaces").join(local_workspace_id);
        let worklog_db = local_workspace_path.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = SessionRuntimeContext {
            trace_id: "trace-worklog".to_string(),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: "session-test".to_string(),
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "GetMessage",
                        "owner_session_id": "session-1",
                        "status": "OK",
                        "payload": {
                            "from": "alice",
                            "channel": "inbox",
                            "snippet": "please check workspace"
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
                        "type": "GetMessage",
                        "owner_session_id": "session-2",
                        "status": "OK",
                        "payload": {
                            "from": "bob",
                            "channel": "inbox",
                            "snippet": "ignore me"
                        }
                    }
                }),
            )
            .await
            .expect("append worklog for session-2");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = local_workspace_path;
        session.local_workspace_id = Some(local_workspace_id.to_string());
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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
        assert!(merged.contains("alice: please check workspace"));
        assert!(merged.contains("please check workspace"));
        assert!(!merged.contains("bob: ignore me"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_falls_back_when_workspace_id_missing_in_record() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let local_workspace_id = "ws-demo";
        let local_workspace_path = agent_env_root.join("workspaces").join(local_workspace_id);
        let worklog_db = local_workspace_path.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = SessionRuntimeContext {
            trace_id: "trace-worklog".to_string(),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: "session-test".to_string(),
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "GetMessage",
                        "owner_session_id": "session-1",
                        "status": "OK",
                        "payload": {
                            "from": "alice",
                            "channel": "inbox",
                            "snippet": "workspace id missing in record"
                        }
                    }
                }),
            )
            .await
            .expect("append worklog");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = local_workspace_path;
        session.local_workspace_id = Some(local_workspace_id.to_string());
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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
            "expected workspace worklog records through session-only fallback"
        );
        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("workspace id missing in record"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_filters_last_step_records() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let local_workspace_id = "ws-demo";
        let local_workspace_path = agent_env_root.join("workspaces").join(local_workspace_id);
        let worklog_db = local_workspace_path.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = SessionRuntimeContext {
            trace_id: "trace-worklog".to_string(),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 2,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: "session-test".to_string(),
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "GetMessage",
                        "owner_session_id": "session-1",
                        "step_id": "step-0",
                        "step_index": 0,
                        "status": "OK",
                        "payload": {
                            "from": "alice",
                            "channel": "inbox",
                            "snippet": "old step record"
                        }
                    }
                }),
            )
            .await
            .expect("append old-step worklog");
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "GetMessage",
                        "owner_session_id": "session-1",
                        "step_id": "step-1",
                        "step_index": 1,
                        "status": "OK",
                        "payload": {
                            "from": "alice",
                            "channel": "inbox",
                            "snippet": "last step record"
                        }
                    }
                }),
            )
            .await
            .expect("append last-step worklog");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = local_workspace_path;
        session.local_workspace_id = Some(local_workspace_id.to_string());
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 2,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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
        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("old step record"));
        assert!(!merged.contains("last step record"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_ignores_tool_call_action_noise() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let local_workspace_id = "ws-demo";
        let local_workspace_path = agent_env_root.join("workspaces").join(local_workspace_id);
        let worklog_db = local_workspace_path.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = SessionRuntimeContext {
            trace_id: "trace-worklog".to_string(),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: "session-test".to_string(),
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "GetMessage",
                        "owner_session_id": "session-1",
                        "status": "OK",
                        "payload": {
                            "from": "alice",
                            "channel": "inbox",
                            "snippet": "keep this message"
                        }
                    }
                }),
            )
            .await
            .expect("append get message");
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "ActionRecord",
                        "owner_session_id": "session-1",
                        "status": "OK",
                        "payload": {
                            "action_type": "tool_call",
                            "cmd_digest": "write_file workspaces/demo/index.html"
                        }
                    }
                }),
            )
            .await
            .expect("append tool_call action");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = local_workspace_path;
        session.local_workspace_id = Some(local_workspace_id.to_string());
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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
        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("keep this message"));
        assert!(!merged.contains("tool_call"));
        assert!(!merged.contains("write_file workspaces/demo/index.html"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_formats_step_header_and_nested_lines() {
        let temp = tempdir().expect("create tempdir");
        let agent_env_root = temp.path().join("workspace");
        let local_workspace_id = "ws-demo";
        let local_workspace_path = agent_env_root.join("workspaces").join(local_workspace_id);
        let worklog_db = local_workspace_path.join("worklog").join("worklog.db");

        let worklog_tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(worklog_db)).expect("create tool");
        let trace_ctx = SessionRuntimeContext {
            trace_id: "trace-worklog".to_string(),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "do".to_string(),
            step_idx: 1,
            wakeup_id: "wakeup-worklog".to_string(),
            session_id: "session-test".to_string(),
        };
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "ActionRecord",
                        "owner_session_id": "session-1",
                        "step_id": "step-1",
                        "step_index": 1,
                        "status": "OK",
                        "behavior": "do",
                        "payload": {
                            "action_type": "bash",
                            "cmd_digest": "cargo test"
                        }
                    }
                }),
            )
            .await
            .expect("append ok action");
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_worklog",
                    "record": {
                        "type": "ActionRecord",
                        "owner_session_id": "session-1",
                        "step_id": "step-1",
                        "step_index": 1,
                        "status": "FAILED",
                        "behavior": "do",
                        "payload": {
                            "action_type": "bash",
                            "cmd_digest": "cargo clippy"
                        }
                    }
                }),
            )
            .await
            .expect("append failed action");
        let _ = worklog_tool
            .call(
                &trace_ctx,
                json!({
                    "action": "append_step_summary",
                    "record": {
                        "type": "StepSummary",
                        "owner_session_id": "session-1",
                        "step_id": "step-1",
                        "step_index": 1,
                        "behavior": "do",
                        "payload": {
                            "did_digest": "plan",
                            "result_digest": "## Step 1 / 16 Summary: done"
                        }
                    }
                }),
            )
            .await
            .expect("append step summary");

        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session.pwd = local_workspace_path;
        session.local_workspace_id = Some(local_workspace_id.to_string());
        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-1".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 3,
                wakeup_id: "wakeup-1".to_string(),
                session_id: "session-test".to_string(),
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

        let records = load_workspace_worklog_with_limit(&input, 400, &MockTokenizer).await;
        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        println!("merged:\n{}", merged);
        assert!(merged.contains("### "));
        assert!(merged.contains("SUCCESS (1), FAILED (1)"));
        assert!(merged.contains("Behavior do Step 1 : SUCCESS (1), FAILED (1)"));
        assert!(merged.contains("- Run cargo test => OK"));
        assert!(merged.contains("- Run cargo clippy => FAILED"));
        assert!(!merged.contains("Step completed:"));
    }

    #[tokio::test]
    async fn load_workspace_worklog_with_limit_falls_back_to_session_runtime_memory() {
        let mut session = AgentSession::new("session-1", "did:web:agent.example.com", None);
        session
            .append_worklog(
                json!({
                    "type": "GetMessage",
                    "status": "OK",
                    "payload": {
                        "from": "alice",
                        "channel": "inbox",
                        "snippet": "runtime fallback message"
                    }
                }),
                None,
            )
            .await
            .expect("append runtime worklog");

        let input = BehaviorExecInput {
            session_id: "session-1".to_string(),
            trace: SessionRuntimeContext {
                trace_id: "trace-runtime-worklog".to_string(),
                agent_name: "did:web:agent.example.com".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-runtime-worklog".to_string(),
                session_id: "session-test".to_string(),
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
            "expected runtime worklog fallback records"
        );
        assert_eq!(records[0].source_label, "worklog");

        let merged = records
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("alice: runtime fallback message"));
        assert!(merged.contains("runtime fallback message"));
    }
}
