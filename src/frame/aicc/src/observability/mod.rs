use serde_json::Value;

const REDACTED: &str = "[REDACTED]";

pub(crate) fn redact(value: &Value) -> Value {
    redact_at(value, None)
}

fn redact_at(value: &Value, key: Option<&str>) -> Value {
    if key.is_some_and(is_sensitive_key) {
        return Value::String(REDACTED.into());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), redact_at(value, Some(key))))
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(|value| redact_at(value, key)).collect())
        }
        Value::String(text) => Value::String(redact_inline_secrets(text)),
        _ => value.clone(),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "token",
        "secret",
        "password",
        "private_key",
        "credential",
        "prompt",
        "content",
        "input",
        "output",
        "request_body",
        "response_body",
    ]
    .iter()
    .any(|sensitive| key == *sensitive || key.ends_with(&format!("_{sensitive}")))
}

fn redact_inline_secrets(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(words.len());
    let mut hide_next = false;
    for word in words {
        if hide_next {
            output.push(REDACTED);
            hide_next = false;
            continue;
        }
        if word.eq_ignore_ascii_case("bearer") {
            output.push(word);
            hide_next = true;
        } else if looks_like_secret(word) {
            output.push(REDACTED);
        } else {
            output.push(word);
        }
    }
    output.join(" ")
}

fn looks_like_secret(value: &str) -> bool {
    let value = value.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '-' && character != '_'
    });
    (value.starts_with("sk-")
        || value.starts_with("sk_")
        || value.starts_with("sess-")
        || value.starts_with("pk-"))
        && value.len() >= 12
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan_sensitive(value: &Value) -> Vec<String> {
        let mut findings = Vec::new();
        scan_at(value, "$", None, &mut findings);
        findings
    }

    fn scan_at(value: &Value, path: &str, key: Option<&str>, findings: &mut Vec<String>) {
        if key.is_some_and(is_sensitive_key) && value != REDACTED {
            findings.push(path.into());
        }
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    scan_at(value, &format!("{path}.{key}"), Some(key), findings);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    scan_at(value, &format!("{path}[{index}]"), key, findings);
                }
            }
            Value::String(text) if text != REDACTED => {
                let lower = text.to_ascii_lowercase();
                if text.split_whitespace().any(looks_like_secret)
                    || (lower.contains("bearer ") && !lower.contains("bearer [redacted]"))
                {
                    findings.push(path.into());
                }
            }
            _ => {}
        }
    }

    #[test]
    fn redaction_removes_credentials_and_content_but_keeps_correlation() {
        let original = json!({"request_id":"req-1", "authorization":"Bearer very-secret-token", "prompt":"private question",
            "nested":{"api_key":"sk-123456789012345", "message":"failed with Bearer abcdefghijklmnop"}});
        assert!(!scan_sensitive(&original).is_empty());
        let safe = redact(&original);
        assert_eq!(safe["request_id"], "req-1");
        assert_eq!(safe["authorization"], REDACTED);
        assert_eq!(safe["prompt"], REDACTED);
        assert_eq!(safe["nested"]["api_key"], REDACTED);
        assert!(scan_sensitive(&safe).is_empty());
    }
}
