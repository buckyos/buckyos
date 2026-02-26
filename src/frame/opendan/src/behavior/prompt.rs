use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value as Json;
use tokio::sync::Mutex;

use crate::agent_enviroment::AgentEnvironment;
use crate::agent_session::AgentSession;
use crate::agent_tool::ToolSpec;
use crate::behavior::BehaviorConfig;

use super::sanitize::{sanitize_json_compact, sanitize_text};
use super::types::{BehaviorExecInput, LLMBehaviorConfig};
use super::Tokenizer;

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
        cfg: &BehaviorConfig,
        tokenizer: &dyn Tokenizer,
        session: Option<Arc<Mutex<AgentSession>>>,
    ) -> Result<PromptPack, String> {
        let env_context = build_env_context(input);

        let process_rules = render_section(
            cfg.process_rule.as_str(),
            &env_context,
            session.clone(),
        )
        .await?;

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
            system_parts.push(format!("<<policy>>\n{}\n<</policy>>", sanitize_text(policy_text.as_str())));
        }
        system_parts.push(format!(
            "<<output_protocol>>\n{}\n<</output_protocol>>",
            sanitize_text(output_protocol_text.as_str())
        ));

        if let Some(toolbox) = build_toolbox(tools, cfg) {
            system_parts.push(format!("<<toolbox>>\n{}\n<</toolbox>>", toolbox));
        }

        let system_role_prompt_text = system_parts.join("\n\n");

        let memory_prompt_text = build_memory_prompt_text(
            input,
            &system_role_prompt_text,
            tokenizer,
        )
        .await;

        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: system_role_prompt_text,
                name: None,
            },
            ChatMessage {
                role: ChatRole::User,
                content: memory_prompt_text,
                name: None,
            },
            ChatMessage {
                role: ChatRole::User,
                content: format!("<<input>>\n{}\n<</input>>", input.input_prompt),
                name: None,
            },
        ];
        Ok(PromptPack { messages })
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
    ctx.insert("step.index".to_string(), Json::String(input.trace.step_idx.to_string()));
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

/// Build memory prompt with dynamic compression. Empty implementation for now.
async fn build_memory_prompt_text(
    _input: &BehaviorExecInput,
    _system_prompt: &str,
    _tokenizer: &dyn Tokenizer,
) -> String {
    // TODO: dynamic compression based on token budget
    String::new()
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

fn build_toolbox(tools: &[ToolSpec], cfg: &BehaviorConfig) -> Option<String> {
    let filtered = cfg.toolbox.tools.filter_tool_specs(tools);
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
            last_pulled_msg_index: 0,
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

        let prompt = PromptBuilder::build(
            &input,
            &[],
            &input.behavior_cfg,
            &MockTokenizer,
            None,
        )
        .await
        .expect("build prompt");

        let system = prompt
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
