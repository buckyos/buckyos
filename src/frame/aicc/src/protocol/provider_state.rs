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
            if object.get("type").and_then(Value::as_str) == Some("provider_state") {
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
            normalized_base_url: "https://gateway.example/v1".to_string(),
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
    }
}
