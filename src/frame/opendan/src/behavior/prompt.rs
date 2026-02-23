use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value as Json;

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
    pub fn build(
        input: &BehaviorExecInput,
        tools: &[ToolSpec],
        cfg: &LLMBehaviorConfig,
        tokenizer: &dyn Tokenizer,
    ) -> Result<PromptPack, String> {
        let mut system_sections = vec![
            format!("<<role>>\n{}\n<</role>>", build_role_section(input)),
            format!(
                "<<process_rules>>\n{}\n<</process_rules>>",
                sanitize_text(&input.behavior_prompt)
            ),
        ];

        if let Some(policy_text) = build_policy_text(input) {
            system_sections.push(format!("<<policy>>\n{}\n<</policy>>", policy_text));
        }
        if let Some(input_template) = build_input_template(input) {
            system_sections.push(format!(
                "<<input_template>>\n{}\n<</input_template>>",
                input_template
            ));
        }
        if let Some(memory_policy) = build_memory_policy(input) {
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

        let input_section = format!(
            "<<Input>>\n{}\n<</Input>>",
            sanitize_json_compact(&input.inbox)
        );

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

fn build_policy_text(input: &BehaviorExecInput) -> Option<String> {
    lookup_env(input, "behavior.policy")
}

fn build_input_template(input: &BehaviorExecInput) -> Option<String> {
    lookup_env(input, "behavior.input_template")
}

fn build_memory_policy(input: &BehaviorExecInput) -> Option<String> {
    lookup_env(input, "behavior.memory_config")
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
    match cfg.output_mode.trim().to_ascii_lowercase().as_str() {
        "behavior_llm_result" | "json_v1" | "executor" => {
            "Output mode: behavior_llm_result".to_string()
        }
        "route_result" | "route" => "Output mode: route_result".to_string(),
        _ => String::new(),
    }
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
    let Some(raw) = lookup_env(input, "behavior.toolbox_skills") else {
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
