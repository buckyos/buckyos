use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{
    features, AiMessage, AiPayload, AiToolSpec, Capability, CompleteRequest, ModelSpec,
    Requirements,
};
use chrono::{DateTime, Utc};
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::sync::Mutex;
use tokio::task;

use crate::agent_enviroment::AgentEnvironment;
use crate::agent_session::AgentSession;
use crate::behavior::config::BehaviorMemoryBucketConfig;
use crate::behavior::BehaviorConfig;
use crate::workspace::todo::render_workspace_todo_prompt_from_db;

use super::sanitize::{sanitize_json_compact, sanitize_text};
use super::types::{BehaviorExecInput, LLMBehaviorConfig};
use super::Tokenizer;

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
    pub async fn build(
        input: &BehaviorExecInput,
        tools: &[AiToolSpec],
        cfg: &BehaviorConfig,
        tokenizer: &dyn Tokenizer,
        session: Option<Arc<Mutex<AgentSession>>>,
    ) -> Result<CompleteRequest, String> {
        let env_context = build_env_context(input);

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

        let output_protocol_text = {
            let raw = cfg.llm.output_protocol.as_str();
            if raw.trim().is_empty() {
                build_output_protocol(&cfg.llm)
            } else {
                render_section(raw, &env_context, session.clone()).await?
            }
        };

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

        if let Some(toolbox) = build_toolbox(tools, cfg) {
            system_parts.push(format!("<<toolbox>>\n{}\n<</toolbox>>", toolbox));
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
            build_memory_prompt_text(input, cfg, vec![], memory_token_limit, tokenizer).await;

        let ai_messages: Vec<AiMessage> = vec![
            AiMessage::new("system".to_string(), system_role_prompt_text),
            AiMessage::new("user".to_string(), memory_prompt_text),
            AiMessage::new(
                "user".to_string(),
                format!("<<input>>\n{}\n<</input>>", input.input_prompt),
            ),
        ];

        let mut must_features = vec![features::JSON_OUTPUT.to_string()];
        if !tools.is_empty() {
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
                tools.to_vec(),
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
            lines.push(format!(
                "{}",
                sanitize_json_compact(workspace_info)
            ));
        }
    }

    fit_text_with_token_limit(lines.join("\n"), token_limit, tokenizer)
}

async fn load_agent_memory_with_limit(
    input: &BehaviorExecInput,
    topic_tags: &[String],
    token_limit: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
   unimplemented!();
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

fn collect_workspace_path_candidates(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::<PathBuf>::new();
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
    _input: &BehaviorExecInput,
    _token_limit: u32,
    _tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    vec![]
}

async fn load_workspace_worklog_with_limit(
    _input: &BehaviorExecInput,
    _token_limit: u32,
    _tokenizer: &dyn Tokenizer,
) -> Vec<MemoryTimelineRecord> {
    vec![]
}

fn build_output_protocol(cfg: &LLMBehaviorConfig) -> String {
    if !cfg.output_protocol.trim().is_empty() {
        return cfg.output_protocol.clone();
    }
    match normalize_output_mode(cfg.output_mode.as_str()).as_str() {
        "behavior_llm_result" => build_behavior_llm_result_protocol(),
        _ => build_auto_output_protocol(),
    }
}

fn normalize_output_mode(mode: &str) -> String {
    let normalized = mode.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "auto" => "auto".to_string(),
        "json_v1" | "behavior_llm_result" | "behavior_result" | "executor" => {
            "behavior_llm_result".to_string()
        }
        "route_result" | "route" | "route_v1" => "behavior_llm_result".to_string(),
        _ => "auto".to_string(),
    }
}

fn build_behavior_llm_result_protocol() -> String {
    let schema = json!({
        "next_behavior": "END",
        "thinking": "optional short reasoning for internal planning",
        "reply": [
            {
                "audience": "user",
                "format": "markdown",
                "content": "final response for user"
            }
        ],
        "todo": [
            {
                "op": "upsert",
                "id": "T001"
            }
        ],
        "todo_delta": {
            "ops": [
                {
                    "op": "update:T001",
                    "to_status": "DONE"
                }
            ]
        },
        "set_memory": [
            {
                "scope": "session",
                "key": "summary",
                "value": "important context"
            }
        ],
        "actions": [
            {
                "kind": "bash",
                "title": "Run tests",
                "command": "cargo test -p opendan",
                "execution_mode": "serial",
                "cwd": ".",
                "timeout_ms": 120000,
                "allow_network": false,
                "fs_scope": {
                    "read_roots": [
                        "."
                    ],
                    "write_roots": [
                        "."
                    ]
                },
                "rationale": "verify the change"
            }
        ],
        "session_delta": [
            {
                "key": "status",
                "value": "updated"
            }
        ],
        "is_sleep": false,
        "output": {
            "kind": "final"
        }
    });
    let schema_pretty = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string());

    format!(
        "Return ONLY one JSON object. No markdown fences and no extra text.\n\
Output mode: behavior_llm_result\n\
Allowed top-level keys only: next_behavior, thinking, reply, todo, todo_delta, set_memory, actions, session_delta, is_sleep, output.\n\
Type rules:\n\
- `reply` is array of objects with `audience`, `format`, `content`.\n\
- `actions` is array of ActionSpec objects; each action requires `title` and `command`.\n\
- `todo_delta` is legacy alias of `todo`; use `todo` in new outputs.\n\
JSON example:\n\
{}",
        schema_pretty
    )
}

fn build_auto_output_protocol() -> String {
    format!(
        "Output mode: auto.\n\
Return ONLY one JSON object and use behavior_llm_result schema:\n\n\
[behavior_llm_result]\n\
{}",
        build_behavior_llm_result_protocol()
    )
}

fn build_toolbox(tools: &[AiToolSpec], cfg: &BehaviorConfig) -> Option<String> {
    let filtered = cfg.toolbox.tools.filter_ai_tool_specs(tools);
    let skills = cfg.toolbox.skills.clone();
    if filtered.is_empty() && skills.is_empty() {
        return None;
    }
    let value = json!({
        "tools": filtered,
        "skills": skills,
    });
    Some(sanitize_json_compact(&value))
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
    use super::*;
    use crate::behavior::types::{StepLimits, TraceCtx};

    struct MockTokenizer;

    impl Tokenizer for MockTokenizer {
        fn count_tokens(&self, text: &str) -> u32 {
            text.split_whitespace().count() as u32
        }
    }

    #[test]
    fn build_output_protocol_uses_explicit_text_when_provided() {
        let cfg = LLMBehaviorConfig {
            output_protocol: "custom protocol".to_string(),
            ..Default::default()
        };
        assert_eq!(build_output_protocol(&cfg), "custom protocol");
    }

    #[test]
    fn build_output_protocol_behavior_mode_uses_behavior_schema() {
        let cfg = LLMBehaviorConfig {
            output_mode: "behavior_result".to_string(),
            ..Default::default()
        };
        let protocol = build_output_protocol(&cfg);
        assert!(protocol.contains("Output mode: behavior_llm_result"));
        assert!(protocol.contains("\"actions\""));
        assert!(protocol.contains("\"reply\""));
    }

    #[test]
    fn build_output_protocol_route_mode_alias_uses_behavior_schema() {
        let cfg = LLMBehaviorConfig {
            output_mode: "route_v1".to_string(),
            ..Default::default()
        };
        let protocol = build_output_protocol(&cfg);
        assert!(protocol.contains("Output mode: behavior_llm_result"));
        assert!(protocol.contains("\"actions\""));
        assert!(protocol.contains("\"reply\""));
    }

    #[test]
    fn build_output_protocol_auto_mode_lists_behavior_schema_only() {
        let cfg = LLMBehaviorConfig {
            output_mode: "auto".to_string(),
            ..Default::default()
        };
        let protocol = build_output_protocol(&cfg);
        assert!(protocol.contains("[behavior_llm_result]"));
        assert!(!protocol.contains("[route_result]"));
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

        let req = PromptBuilder::build(&input, &[], &input.behavior_cfg, &MockTokenizer, None)
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
}
