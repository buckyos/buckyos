use serde_json::Value;

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
    use super::foreign_provider_state_text;
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
}
