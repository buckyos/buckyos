use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value as Json;

use crate::agent_enviroment::{AgentEnvironment, PromptTemplateContext, TemplateRenderMode};
use crate::agent_tool::ToolSpec;

use super::sanitize::{sanitize_json_compact, sanitize_text};
use super::types::{BehaviorExecInput, LLMBehaviorConfig};
use super::{Sanitizer, Tokenizer};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PromptPack {
    pub messages: Vec<ChatMessage>,
}

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
        tools: &[ToolSpec],
        cfg: &LLMBehaviorConfig,
        tokenizer: &dyn Tokenizer,
        environment: &AgentEnvironment,
    ) -> Result<PromptPack, String> {
        let render_ctx = build_render_context(input, environment);
        let process_rules = render_text_template(
            input.behavior_prompt.as_str(),
            &render_ctx,
            environment,
            TemplateRenderMode::Text,
        )
        .await?;

        let mut system_sections = vec![
            format!("<<role>>\n{}\n<</role>>", build_role_section(input)),
            format!(
                "<<process_rules>>\n{}\n<</process_rules>>",
                sanitize_text(process_rules.as_str())
            ),
        ];

        if let Some(policy_text) = build_policy_text(&render_ctx, environment).await? {
            system_sections.push(format!("<<policy>>\n{}\n<</policy>>", policy_text));
        }

        if let Some(memory_policy) = build_memory_policy(&render_ctx, environment).await? {
            system_sections.push(format!(
                "<<memory_policy>>\n{}\n<</memory_policy>>",
                memory_policy
            ));
        }
        if let Some(step_hints) = build_step_hints(input) {
            system_sections.push(format!("<<step_hints>>\n{}\n<</step_hints>>", step_hints));
        }
        if let Some(toolbox) = build_toolbox(tools, input) {
            system_sections.push(format!("<<toolbox>>\n{}\n<</toolbox>>", toolbox));
        }

        system_sections.push(format!(
            "<<output_protocol>>\n{}\n<</output_protocol>>",
            build_output_protocol(cfg)
        ));
        let system = system_sections.join("\n\n");

        let input_text = build_input_text(input, &render_ctx, environment).await?;
        let input_section = format!("<<Input>>\n{}\n<</Input>>", input_text);

        let memory = if is_empty_like_json(&input.memory) {
            None
        } else {
            Some(format!(
                "<<Memory>>\n{}\n<</Memory>>",
                sanitize_json_compact(&input.memory)
            ))
        };

        let observations = if input.last_observations.is_empty() {
            None
        } else {
            Some(format!(
                "<<Observations>>\n{}\n<</Observations>>",
                Sanitizer::format_observations(
                    &input.last_observations,
                    input.limits.max_observation_bytes
                )
            ))
        };

        let mut messages = vec![ChatMessage {
            role: ChatRole::System,
            name: None,
            content: system,
        }];

        if let Some(memory) = memory {
            messages.push(ChatMessage {
                role: ChatRole::User,
                name: None,
                content: memory,
            });
        }

        messages.push(ChatMessage {
            role: ChatRole::User,
            name: None,
            content: input_section,
        });

        if let Some(obs) = observations {
            messages.push(ChatMessage {
                role: ChatRole::User,
                name: None,
                content: obs,
            });
        }

        let messages =
            Truncator::fit_into_budget(messages, input.limits.max_prompt_tokens, tokenizer);
        Ok(PromptPack { messages })
    }
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

fn build_role_section(input: &BehaviorExecInput) -> String {
    let role = sanitize_text(&input.role_md);
    let self_desc = sanitize_text(&input.self_md);
    if self_desc.is_empty() {
        return role;
    }
    format!("{}\n\n[Self]\n{}", role, self_desc)
}

fn build_render_context(
    input: &BehaviorExecInput,
    environment: &AgentEnvironment,
) -> PromptTemplateContext {
    let cwd_path = lookup_env(input, "cwd.path").map(std::path::PathBuf::from);
    let mut ctx = environment.build_prompt_template_context(&input.inbox, cwd_path);
    let session_id = input
        .session_id
        .clone()
        .or_else(|| lookup_env(input, "loop.session_id"));
    ctx.session_id = session_id.clone();

    for kv in &input.env_context {
        if kv.key.trim().is_empty() {
            continue;
        }
        ctx.runtime_kv
            .insert(kv.key.clone(), Json::String(kv.value.clone()));
    }
    if let Some(session_id) = session_id {
        ctx.runtime_kv
            .entry("session_id".to_string())
            .or_insert_with(|| Json::String(session_id.clone()));
        ctx.runtime_kv
            .entry("loop.session_id".to_string())
            .or_insert_with(|| Json::String(session_id));
    }

    ctx
}

async fn render_text_template(
    template: &str,
    ctx: &PromptTemplateContext,
    environment: &AgentEnvironment,
    mode: TemplateRenderMode,
) -> Result<String, String> {
    let rendered = environment
        .render_prompt_template(template, mode, ctx)
        .await
        .map_err(|err| err.to_string())?
        .unwrap_or_default();
    Ok(sanitize_text(rendered.as_str()))
}

async fn build_policy_text(
    ctx: &PromptTemplateContext,
    environment: &AgentEnvironment,
) -> Result<Option<String>, String> {
    let Some(template) = lookup_runtime_kv(ctx, "policy.text") else {
        return Ok(None);
    };
    let rendered = render_text_template(
        template.as_str(),
        ctx,
        environment,
        TemplateRenderMode::Text,
    )
    .await?;
    if rendered.is_empty() {
        return Ok(None);
    }
    Ok(Some(rendered))
}

async fn build_memory_policy(
    ctx: &PromptTemplateContext,
    environment: &AgentEnvironment,
) -> Result<Option<String>, String> {
    let Some(template) = lookup_runtime_kv(ctx, "memory.policy") else {
        return Ok(None);
    };
    let rendered = render_text_template(
        template.as_str(),
        ctx,
        environment,
        TemplateRenderMode::Text,
    )
    .await?;
    if rendered.is_empty() {
        return Ok(None);
    }
    Ok(Some(rendered))
}

async fn build_input_text(
    input: &BehaviorExecInput,
    ctx: &PromptTemplateContext,
    environment: &AgentEnvironment,
) -> Result<String, String> {
    let Some(template) = lookup_runtime_kv(ctx, "input.template") else {
        return Ok(sanitize_json_compact(&input.inbox));
    };

    let rendered = environment
        .render_prompt_template(template.as_str(), TemplateRenderMode::InputBlock, ctx)
        .await
        .map_err(|err| err.to_string())?;
    if let Some(text) = rendered {
        let compact = sanitize_text(text.as_str());
        if !compact.is_empty() {
            return Ok(compact);
        }
    }

    Ok(sanitize_json_compact(&input.inbox))
}

fn build_step_hints(input: &BehaviorExecInput) -> Option<String> {
    let hints = input
        .env_context
        .iter()
        .filter(|kv| kv.key.starts_with("step.") || kv.key.starts_with("loop."))
        .map(|kv| format!("{}: {}", kv.key, kv.value))
        .collect::<Vec<_>>();
    if hints.is_empty() {
        return None;
    }
    Some(hints.join("\n"))
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

fn build_toolbox(tools: &[ToolSpec], input: &BehaviorExecInput) -> Option<String> {
    let skills = extract_toolbox_skills(input);
    if tools.is_empty() && skills.is_empty() {
        return None;
    }
    let value = json!({
        "tools": tools,
        "skills": skills,
    });
    Some(sanitize_json_compact(&value))
}

fn extract_toolbox_skills(input: &BehaviorExecInput) -> Vec<String> {
    let Some(raw) = lookup_env(input, "toolbox.skills") else {
        return vec![];
    };
    let parsed = serde_json::from_str::<Json>(&raw).ok();
    if let Some(Json::Array(values)) = parsed {
        return values
            .iter()
            .filter_map(|v| v.as_str().map(str::trim))
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .collect::<Vec<_>>();
    }

    raw.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
}

fn lookup_env(input: &BehaviorExecInput, key: &str) -> Option<String> {
    input
        .env_context
        .iter()
        .find(|kv| kv.key == key)
        .map(|kv| sanitize_text(&kv.value))
        .filter(|v| !v.is_empty())
}

fn lookup_runtime_kv(ctx: &PromptTemplateContext, key: &str) -> Option<String> {
    let value = ctx.runtime_kv.get(key)?;
    match value {
        Json::String(v) => {
            let text = sanitize_text(v);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => {
            let text = sanitize_text(serde_json::to_string(value).unwrap_or_default().as_str());
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }
}

fn contains_any_marker(content: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| content.contains(marker))
}

fn is_empty_like_json(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::String(v) => v.trim().is_empty(),
        Json::Array(values) => values.is_empty() || values.iter().all(is_empty_like_json),
        Json::Object(map) => map.is_empty() || map.values().all(is_empty_like_json),
        Json::Bool(_) | Json::Number(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::behavior::types::{EnvKV, StepLimits, TraceCtx};

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
    async fn build_renders_policy_template_with_session_and_env_context() {
        let root = tempdir().expect("create tempdir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create environment");
        let input = BehaviorExecInput {
            session_id: Some("session-1".to_string()),
            trace: TraceCtx {
                trace_id: "trace-1".to_string(),
                agent_did: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 2,
                wakeup_id: "wakeup-1".to_string(),
            },
            role_md: "role".to_string(),
            self_md: "self".to_string(),
            behavior_prompt: "rules".to_string(),
            env_context: vec![
                EnvKV {
                    key: "policy.text".to_string(),
                    value: "sid={{session_id}}, step={{step.index}}".to_string(),
                },
                EnvKV {
                    key: "step.index".to_string(),
                    value: "2".to_string(),
                },
            ],
            inbox: json!({"new_msg":"hello"}),
            memory: json!({}),
            last_observations: vec![],
            limits: StepLimits::default(),
            behavior_cfg: Default::default(),
        };

        let prompt = PromptBuilder::build(
            &input,
            &[],
            &LLMBehaviorConfig::default(),
            &MockTokenizer,
            &env,
        )
        .await
        .expect("build prompt");
        let system = prompt
            .messages
            .first()
            .map(|msg| msg.content.clone())
            .unwrap_or_default();

        assert!(system.contains("<<policy>>"));
        assert!(system.contains("sid=session-1, step=2"));
    }

    #[tokio::test]
    async fn build_uses_rendered_input_template_when_configured() {
        let root = tempdir().expect("create tempdir");
        let env = AgentEnvironment::new(root.path())
            .await
            .expect("create environment");
        let input = BehaviorExecInput {
            session_id: Some("session-2".to_string()),
            trace: TraceCtx {
                trace_id: "trace-2".to_string(),
                agent_did: "did:example:agent".to_string(),
                behavior: "on_wakeup".to_string(),
                step_idx: 1,
                wakeup_id: "wakeup-2".to_string(),
            },
            role_md: "role".to_string(),
            self_md: "self".to_string(),
            behavior_prompt: "rules".to_string(),
            env_context: vec![EnvKV {
                key: "input.template".to_string(),
                value: "{{new_msg}}\n{{step.index}}".to_string(),
            }],
            inbox: json!({"new_msg":"hello"}),
            memory: json!({}),
            last_observations: vec![],
            limits: StepLimits::default(),
            behavior_cfg: Default::default(),
        };

        let prompt = PromptBuilder::build(
            &input,
            &[],
            &LLMBehaviorConfig::default(),
            &MockTokenizer,
            &env,
        )
        .await
        .expect("build prompt");
        let input_msg = prompt
            .messages
            .iter()
            .find(|msg| msg.content.contains("<<Input>>"))
            .map(|msg| msg.content.clone())
            .unwrap_or_default();

        assert!(input_msg.contains("hello"));
        assert!(!input_msg.contains("{\"new_msg\":\"hello\"}"));
    }
}
