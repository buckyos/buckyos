use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const HEALTH_WINDOW_MS: u64 = 5 * 60 * 1_000;
const CIRCUIT_FAILURE_THRESHOLD: u32 = 3;
const CIRCUIT_OPEN_MS: u64 = 30 * 1_000;
const MODEL_UNAVAILABLE_MS: u64 = 5 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HealthFailureKind {
    Transient,
    ModelUnavailable,
    Permanent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModelHealthSnapshot {
    pub model_available: bool,
    pub degraded: bool,
    pub circuit_open: bool,
    pub p50_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub error_rate_5m: Option<f64>,
    pub recent_failures: u32,
}

#[derive(Clone, Debug)]
struct HealthSample {
    at_ms: u64,
    latency_ms: f64,
    failed: bool,
}

#[derive(Clone, Debug, Default)]
struct HealthEntry {
    samples: VecDeque<HealthSample>,
    consecutive_failures: u32,
    circuit_open_until_ms: Option<u64>,
    model_unavailable_until_ms: Option<u64>,
}

#[derive(Clone, Default)]
pub(crate) struct ModelHealthRegistry {
    entries: Arc<Mutex<BTreeMap<String, HealthEntry>>>,
}

impl ModelHealthRegistry {
    pub(crate) fn record_success(
        &self,
        exact_model: &str,
        provider_instance_name: &str,
        latency_ms: f64,
        at_ms: u64,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for key in [model_key(exact_model), provider_key(provider_instance_name)] {
            let entry = entries.entry(key).or_default();
            prune(entry, at_ms);
            entry.samples.push_back(HealthSample {
                at_ms,
                latency_ms: latency_ms.max(0.0),
                failed: false,
            });
            entry.consecutive_failures = 0;
            entry.circuit_open_until_ms = None;
            entry.model_unavailable_until_ms = None;
        }
    }

    pub(crate) fn record_failure(
        &self,
        exact_model: &str,
        provider_instance_name: &str,
        latency_ms: f64,
        kind: HealthFailureKind,
        at_ms: u64,
    ) {
        if kind == HealthFailureKind::Permanent {
            return;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let keys = if kind == HealthFailureKind::ModelUnavailable {
            vec![model_key(exact_model)]
        } else {
            vec![model_key(exact_model), provider_key(provider_instance_name)]
        };
        for key in keys {
            let entry = entries.entry(key).or_default();
            prune(entry, at_ms);
            entry.samples.push_back(HealthSample {
                at_ms,
                latency_ms: latency_ms.max(0.0),
                failed: true,
            });
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            if kind == HealthFailureKind::ModelUnavailable {
                entry.model_unavailable_until_ms = Some(at_ms.saturating_add(MODEL_UNAVAILABLE_MS));
            } else if entry.consecutive_failures >= CIRCUIT_FAILURE_THRESHOLD {
                entry.circuit_open_until_ms = Some(at_ms.saturating_add(CIRCUIT_OPEN_MS));
            }
        }
    }

    pub(crate) fn snapshot(
        &self,
        exact_model: &str,
        provider_instance_name: &str,
        at_ms: u64,
    ) -> ModelHealthSnapshot {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let model = snapshot_entry(entries.get_mut(&model_key(exact_model)), at_ms);
        let provider = snapshot_entry(
            entries.get_mut(&provider_key(provider_instance_name)),
            at_ms,
        );
        ModelHealthSnapshot {
            model_available: model.model_available,
            degraded: model.degraded || provider.degraded,
            circuit_open: model.circuit_open || provider.circuit_open,
            p50_latency_ms: max_option(model.p50_latency_ms, provider.p50_latency_ms),
            p95_latency_ms: max_option(model.p95_latency_ms, provider.p95_latency_ms),
            error_rate_5m: max_option(model.error_rate_5m, provider.error_rate_5m),
            recent_failures: model.recent_failures.max(provider.recent_failures),
        }
    }
}

fn snapshot_entry(entry: Option<&mut HealthEntry>, at_ms: u64) -> ModelHealthSnapshot {
    let Some(entry) = entry else {
        return ModelHealthSnapshot {
            model_available: true,
            ..Default::default()
        };
    };
    prune(entry, at_ms);
    let model_available = entry
        .model_unavailable_until_ms
        .is_none_or(|until| at_ms >= until);
    let circuit_open = entry
        .circuit_open_until_ms
        .is_some_and(|until| at_ms < until);
    let failures = entry.samples.iter().filter(|sample| sample.failed).count();
    let error_rate_5m =
        (!entry.samples.is_empty()).then(|| failures as f64 / entry.samples.len() as f64);
    let mut latencies = entry
        .samples
        .iter()
        .map(|sample| sample.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_by(f64::total_cmp);
    let percentile = |ratio: f64| {
        (!latencies.is_empty()).then(|| {
            let index = ((latencies.len() - 1) as f64 * ratio).ceil() as usize;
            latencies[index]
        })
    };
    ModelHealthSnapshot {
        model_available,
        degraded: !model_available || circuit_open || entry.consecutive_failures > 0,
        circuit_open,
        p50_latency_ms: percentile(0.50),
        p95_latency_ms: percentile(0.95),
        error_rate_5m,
        recent_failures: entry.consecutive_failures,
    }
}

fn prune(entry: &mut HealthEntry, at_ms: u64) {
    let cutoff = at_ms.saturating_sub(HEALTH_WINDOW_MS);
    while entry
        .samples
        .front()
        .is_some_and(|sample| sample.at_ms < cutoff)
    {
        entry.samples.pop_front();
    }
    if entry
        .circuit_open_until_ms
        .is_some_and(|until| at_ms >= until)
    {
        entry.circuit_open_until_ms = None;
    }
    if entry
        .model_unavailable_until_ms
        .is_some_and(|until| at_ms >= until)
    {
        entry.model_unavailable_until_ms = None;
    }
}

fn model_key(exact_model: &str) -> String {
    format!("model:{exact_model}")
}

fn provider_key(provider_instance_name: &str) -> String {
    format!("provider:{provider_instance_name}")
}

fn max_option(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_windowed_health_and_provider_circuit() {
        let health = ModelHealthRegistry::default();
        for (at_ms, latency_ms) in [(1, 100.0), (2, 200.0), (3, 300.0)] {
            health.record_failure(
                "model-a@provider",
                "provider",
                latency_ms,
                HealthFailureKind::Transient,
                at_ms,
            );
        }
        let open = health.snapshot("model-b@provider", "provider", 3);
        assert!(open.circuit_open);
        assert_eq!(open.error_rate_5m, Some(1.0));
        assert_eq!(open.p50_latency_ms, Some(200.0));
        assert_eq!(open.p95_latency_ms, Some(300.0));
        health.record_success("model-a@provider", "provider", 50.0, 4);
        let recovered = health.snapshot("model-b@provider", "provider", 4);
        assert!(!recovered.circuit_open);
        assert_eq!(recovered.recent_failures, 0);
        assert_eq!(recovered.error_rate_5m, Some(0.75));
    }

    #[test]
    fn model_unavailable_is_scoped_to_exact_model() {
        let health = ModelHealthRegistry::default();
        health.record_failure(
            "missing@provider",
            "provider",
            10.0,
            HealthFailureKind::ModelUnavailable,
            1,
        );
        assert!(
            !health
                .snapshot("missing@provider", "provider", 2)
                .model_available
        );
        assert!(
            health
                .snapshot("other@provider", "provider", 2)
                .model_available
        );
        assert!(
            health
                .snapshot("missing@provider", "provider", MODEL_UNAVAILABLE_MS + 1)
                .model_available
        );
    }
}
