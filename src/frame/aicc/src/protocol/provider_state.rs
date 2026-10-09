use serde_json::Value;

pub(crate) fn bind_provider_state_source(
    value: &mut Value,
    source: &buckyos_api::ProviderStateCoordinate,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                bind_provider_state_source(item, source);
            }
        }
        Value::Object(object) => {
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("provider_state" | "thinking")
            ) {
                object.insert(
                    "source".to_string(),
                    serde_json::to_value(source).expect("provider state coordinate serializes"),
                );
            }
            for item in object.values_mut() {
                bind_provider_state_source(item, source);
            }
        }
        _ => {}
    }
}

/// Thinking is only replayable to the provider instance and model that
/// produced it. Returns the request without the thinking blocks of any other
/// (or unbound) source, or `None` when there is nothing to drop. The filter
/// only depends on the coordinates, so the prefix sent to one target stays
/// stable across requests.
pub(crate) fn drop_foreign_thinking(
    call: &buckyos_api::AiccCall,
    target: &buckyos_api::ProviderStateCoordinate,
) -> Option<buckyos_api::AiccCall> {
    use buckyos_api::{AiContent, AiccCall};
    let foreign = |block: &AiContent| matches!(block, AiContent::Thinking { source, .. } if !provider_state_is_native(source, target));
    let messages = match call {
        AiccCall::ChatCompletionsCreate(request) => &request.messages,
        AiccCall::HelperLlmChat(request) => &request.messages,
        _ => return None,
    };
    if !messages
        .iter()
        .any(|message| message.content.iter().any(foreign))
    {
        return None;
    }
    let mut filtered = call.clone();
    let messages = match &mut filtered {
        AiccCall::ChatCompletionsCreate(request) => &mut request.messages,
        AiccCall::HelperLlmChat(request) => &mut request.messages,
        _ => unreachable!("only chat calls carry messages"),
    };
    messages.retain_mut(|message| {
        message.content.retain(|block| !foreign(block));
        !message.content.is_empty()
    });
    Some(filtered)
}

pub(crate) fn provider_state_is_native(
    source: &buckyos_api::ProviderStateCoordinate,
    target: &buckyos_api::ProviderStateCoordinate,
) -> bool {
    source.is_bound() && source == target
}

pub(crate) fn foreign_provider_state_text(provider: &str, value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_public_text(value, &mut parts);
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "Prior {provider} provider state, degraded to text:\n{}",
        parts.join("\n")
    ))
}

fn collect_public_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_public_text(item, parts);
            }
        }
        Value::Object(object) => {
            for key in ["text", "summary", "refusal", "content", "output_text"] {
                if let Some(value) = object.get(key) {
                    collect_public_text(value, parts);
                }
            }
        }
        _ => {}
    }
}

fn push_text(parts: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        parts.push(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bind_provider_state_source, foreign_provider_state_text, provider_state_is_native,
    };
    use buckyos_api::ProviderStateCoordinate;
    use serde_json::json;

    #[test]
    fn extracts_public_provider_state_text() {
        let value = json!({
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": "planned"}],
            "encrypted_content": "secret",
            "content": [{"type": "output_text", "text": "visible"}]
        });
        let text = foreign_provider_state_text("openai", &value).unwrap();
        assert!(text.contains("planned"));
        assert!(text.contains("visible"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn returns_none_for_opaque_provider_state() {
        let value = json!({"type": "reasoning", "encrypted_content": "secret", "id": "rs_1"});
        assert!(foreign_provider_state_text("openai", &value).is_none());
    }

    #[test]
    fn binds_and_compares_the_complete_provider_state_coordinate() {
        let source = ProviderStateCoordinate {
            provider_profile_id: "anthropic".to_string(),
            adapter_type: "openai-responses".to_string(),
            origin_provider: "anthropic".to_string(),
            origin_model: "claude-sonnet".to_string(),
        };
        let mut value = json!({
            "output": [{
                "type": "provider_state",
                "source": ProviderStateCoordinate::unbound(),
                "provider": "openai",
                "value": {"type": "reasoning", "id": "rs_1"}
            }]
        });
        bind_provider_state_source(&mut value, &source);
        assert_eq!(value["output"][0]["source"], json!(source));
        assert!(provider_state_is_native(&source, &source));

        let mut different_adapter = source.clone();
        different_adapter.adapter_type = "claude-messages".to_string();
        assert!(!provider_state_is_native(&source, &different_adapter));

        let mut different_profile = source.clone();
        different_profile.provider_profile_id = "openrouter".to_string();
        assert!(!provider_state_is_native(&source, &different_profile));
    }

    #[test]
    fn binds_thinking_and_drops_only_foreign_thinking() {
        use buckyos_api::{AiContent, AiMessage, AiRole, AiccCall, LlmChatInvokeRequest};

        let source = ProviderStateCoordinate {
            provider_profile_id: "anthropic".to_string(),
            adapter_type: "claude-messages".to_string(),
            origin_provider: "anthropic".to_string(),
            origin_model: "claude-opus".to_string(),
        };
        let mut value = json!({"message": {"role": "assistant", "content": [
            {"type": "thinking", "text": "t", "provider_metadata": {"signature": "s"}},
            {"type": "text", "text": "a"}
        ]}});
        bind_provider_state_source(&mut value, &source);
        let message: AiMessage = serde_json::from_value(value["message"].clone()).unwrap();
        assert!(matches!(
            &message.content[0],
            AiContent::Thinking { source: bound, .. } if bound == &source
        ));

        let call = AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
            "m@p",
            vec![AiMessage::text(AiRole::User, "hi"), message],
        ));
        assert!(super::drop_foreign_thinking(&call, &source).is_none());

        let mut other = source.clone();
        other.provider_profile_id = "openrouter".to_string();
        let AiccCall::ChatCompletionsCreate(filtered) =
            super::drop_foreign_thinking(&call, &other).unwrap()
        else {
            panic!("expected a chat call");
        };
        assert_eq!(filtered.messages.len(), 2);
        assert_eq!(filtered.messages[1].content, vec![AiContent::text("a")]);
    }
}
