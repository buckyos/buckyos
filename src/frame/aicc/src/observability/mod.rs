#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CorrelationIds {
    pub request_id: Option<String>,
    pub task_id: Option<String>,
    pub route_id: Option<String>,
    pub provider_trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetricName {
    RequestLatencyMs,
    RequestError,
    QueueDepth,
    ProviderHealth,
    InventoryRefresh,
    SnapshotGeneration,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct MetricKey {
    name: MetricName,
    labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub(crate) struct MetricValue {
    pub count: u64,
    pub sum: f64,
    pub max: f64,
    pub gauge: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MetricSample {
    pub name: MetricName,
    pub labels: BTreeMap<String, String>,
    pub value: MetricValue,
}

#[derive(Clone, Default)]
pub(crate) struct Metrics {
    values: Arc<RwLock<HashMap<MetricKey, MetricValue>>>,
}

impl Metrics {
    pub(crate) fn record_latency(&self, api_type: &str, provider: &str, latency_ms: u64) {
        self.observe(
            MetricName::RequestLatencyMs,
            labels([("api_type", api_type), ("provider_instance", provider)]),
            latency_ms as f64,
        );
    }

    pub(crate) fn record_error(&self, api_type: &str, provider: Option<&str>, error_code: &str) {
        let mut metric_labels = labels([("api_type", api_type), ("error_code", error_code)]);
        if let Some(provider) = provider {
            metric_labels.insert("provider_instance".into(), provider.into());
        }
        self.increment(MetricName::RequestError, metric_labels);
    }

    pub(crate) fn set_queue_depth(&self, provider: &str, depth: u64) {
        self.gauge(
            MetricName::QueueDepth,
            labels([("provider_instance", provider)]),
            depth as f64,
        );
    }

    pub(crate) fn set_provider_health(&self, provider: &str, healthy: bool) {
        self.gauge(
            MetricName::ProviderHealth,
            labels([("provider_instance", provider)]),
            u8::from(healthy) as f64,
        );
    }

    pub(crate) fn record_refresh(&self, provider: &str, outcome: &str, latency_ms: u64) {
        self.observe(
            MetricName::InventoryRefresh,
            labels([("provider_instance", provider), ("outcome", outcome)]),
            latency_ms as f64,
        );
    }

    pub(crate) fn record_snapshot_generation(&self, generation: u64) {
        self.gauge(
            MetricName::SnapshotGeneration,
            BTreeMap::new(),
            generation as f64,
        );
    }

    pub(crate) fn snapshot(&self) -> Vec<MetricSample> {
        let values = self
            .values
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let mut samples = values
            .iter()
            .map(|(key, value)| MetricSample {
                name: key.name,
                labels: key.labels.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        samples.sort_by(|left, right| (left.name, &left.labels).cmp(&(right.name, &right.labels)));
        samples
    }

    fn increment(&self, name: MetricName, labels: BTreeMap<String, String>) {
        let mut values = self
            .values
            .write()
            .unwrap_or_else(|error| error.into_inner());
        values.entry(MetricKey { name, labels }).or_default().count += 1;
    }

    fn observe(&self, name: MetricName, labels: BTreeMap<String, String>, value: f64) {
        let mut values = self
            .values
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let metric = values.entry(MetricKey { name, labels }).or_default();
        metric.count += 1;
        metric.sum += value;
        metric.max = metric.max.max(value);
    }

    fn gauge(&self, name: MetricName, labels: BTreeMap<String, String>, value: f64) {
        self.values
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .entry(MetricKey { name, labels })
            .or_default()
            .gauge = Some(value);
    }
}

fn labels<const N: usize>(values: [(&str, &str); N]) -> BTreeMap<String, String> {
    values
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SensitiveFinding {
    pub path: String,
    pub category: &'static str,
}

pub(crate) fn scan_sensitive(value: &Value) -> Vec<SensitiveFinding> {
    let mut findings = Vec::new();
    scan_at(value, "$", None, &mut findings);
    findings
}

fn scan_at(value: &Value, path: &str, key: Option<&str>, findings: &mut Vec<SensitiveFinding>) {
    if key.is_some_and(is_sensitive_key) && value != REDACTED {
        findings.push(SensitiveFinding {
            path: path.into(),
            category: "sensitive_field",
        });
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
                findings.push(SensitiveFinding {
                    path: path.into(),
                    category: "secret_pattern",
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metrics_cover_required_dimensions_without_correlation_cardinality() {
        let metrics = Metrics::default();
        metrics.record_latency("llm", "openai-primary", 20);
        metrics.record_latency("llm", "openai-primary", 30);
        metrics.record_error("llm", Some("openai-primary"), "provider_error");
        metrics.set_queue_depth("openai-primary", 4);
        metrics.set_provider_health("openai-primary", true);
        metrics.record_refresh("openai-primary", "succeeded", 15);
        metrics.record_snapshot_generation(7);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.len(), 6);
        let latency = snapshot
            .iter()
            .find(|sample| sample.name == MetricName::RequestLatencyMs)
            .unwrap();
        assert_eq!(latency.value.count, 2);
        assert_eq!(latency.value.sum, 50.0);
        assert_eq!(latency.value.max, 30.0);
        assert!(snapshot.iter().all(|sample| !sample
            .labels
            .keys()
            .any(|key| key.contains("request_id") || key.contains("trace_id"))));
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

    #[test]
    fn correlation_ids_round_trip_without_payload_data() {
        let ids = CorrelationIds {
            request_id: Some("request-1".into()),
            task_id: Some("task-1".into()),
            route_id: Some("route-1".into()),
            provider_trace_id: Some("provider-1".into()),
        };
        assert_eq!(
            serde_json::from_value::<CorrelationIds>(serde_json::to_value(&ids).unwrap()).unwrap(),
            ids
        );
    }
}
