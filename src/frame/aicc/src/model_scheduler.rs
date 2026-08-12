use crate::model_types::{
    ModelCandidate, PreferenceScoreInputs, ProviderType, RankedCandidateTrace, RoutePolicy,
    RoutePricingSnapshot, SchedulerProfile, SchedulerProfileWeights, ScoreInputs,
};
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub struct ScheduledRoute {
    pub selected: ModelCandidate,
    pub ranked_candidates: Vec<RankedCandidateTrace>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelScheduler;

impl ModelScheduler {
    pub fn schedule(
        &self,
        candidates: &[ModelCandidate],
        policy: &RoutePolicy,
    ) -> Option<ScheduledRoute> {
        if candidates.is_empty() {
            return None;
        }

        let scored = score_candidates(candidates, policy);
        let selected = scored
            .iter()
            .min_by(|left, right| {
                left.score
                    .partial_cmp(&right.score)
                    .unwrap_or(Ordering::Equal)
            })
            .map(|item| item.candidate.clone())?;

        Some(ScheduledRoute {
            selected: selected.clone(),
            ranked_candidates: ranked_trace(candidates, Some(&selected), Some(&scored)),
        })
    }
}

#[derive(Clone, Debug)]
struct ScoredCandidate {
    candidate: ModelCandidate,
    score: f64,
    inputs: ScoreInputs,
}

fn score_candidates(candidates: &[ModelCandidate], policy: &RoutePolicy) -> Vec<ScoredCandidate> {
    let costs = value_range(candidates.iter().map(cost_value));
    let latencies = value_range(candidates.iter().map(latency_value));
    let risks = value_range(candidates.iter().map(risk_value));
    let qualities = value_range(candidates.iter().map(quality_penalty));
    let preferences = value_range(candidates.iter().map(preference_penalty));

    candidates
        .iter()
        .cloned()
        .map(|candidate| {
            let cost = normalize(cost_value(&candidate), costs);
            let latency = normalize(latency_value(&candidate), latencies);
            let risk = normalize(risk_value(&candidate), risks);
            let quality = normalize(quality_penalty(&candidate), qualities);
            let local_penalty =
                if candidate.metadata.attributes.provider_type == ProviderType::LocalInference {
                    0.0
                } else {
                    1.0
                };

            let weights = policy
                .scheduler_profiles
                .as_ref()
                .and_then(|profiles| profiles.weights_for(&policy.profile))
                .cloned()
                .unwrap_or_else(|| default_weights(&policy.profile));
            let preference = normalize(preference_penalty(&candidate), preferences);
            let cache = 0.0;
            let inputs = ScoreInputs {
                cost,
                latency,
                reliability: risk,
                quality,
                preference,
                cache,
                local: local_penalty,
            };
            let score = weights.cost * inputs.cost
                + weights.latency * inputs.latency
                + weights.reliability * inputs.reliability
                + weights.quality * inputs.quality
                + weights.preference * inputs.preference
                + weights.cache * inputs.cache
                + weights.local * inputs.local;

            ScoredCandidate {
                candidate,
                score,
                inputs,
            }
        })
        .collect()
}

fn default_weights(profile: &SchedulerProfile) -> SchedulerProfileWeights {
    match profile {
        SchedulerProfile::CostFirst => SchedulerProfileWeights {
            cost: 0.55,
            latency: 0.15,
            reliability: 0.15,
            quality: 0.10,
            preference: 0.05,
            cache: 0.10,
            local: 0.0,
        },
        SchedulerProfile::LatencyFirst => SchedulerProfileWeights {
            cost: 0.20,
            latency: 0.45,
            reliability: 0.20,
            quality: 0.10,
            preference: 0.05,
            cache: 0.10,
            local: 0.0,
        },
        SchedulerProfile::QualityFirst => SchedulerProfileWeights {
            cost: 0.15,
            latency: 0.10,
            reliability: 0.20,
            quality: 0.50,
            preference: 0.05,
            cache: 0.10,
            local: 0.0,
        },
        SchedulerProfile::Balanced => SchedulerProfileWeights {
            cost: 0.25,
            latency: 0.25,
            reliability: 0.25,
            quality: 0.25,
            preference: 0.05,
            cache: 0.10,
            local: 0.0,
        },
        SchedulerProfile::LocalFirst => SchedulerProfileWeights {
            cost: 0.12,
            latency: 0.08,
            reliability: 0.0,
            quality: 0.10,
            preference: 0.0,
            cache: 0.0,
            local: 0.70,
        },
        SchedulerProfile::StrictLocal => SchedulerProfileWeights {
            cost: 0.12,
            latency: 0.08,
            reliability: 0.0,
            quality: 0.10,
            preference: 0.0,
            cache: 0.0,
            local: 10.0,
        },
    }
}

fn cost_value(candidate: &ModelCandidate) -> f64 {
    if let Some(estimate) = candidate.dynamic_cost_estimate.as_ref() {
        return estimate.estimated_cost_usd.max(0.0);
    }
    candidate
        .metadata
        .pricing
        .estimated_cost
        .unwrap_or_else(|| match candidate.metadata.attributes.cost_class {
            crate::model_types::CostClass::Low => 0.1,
            crate::model_types::CostClass::Medium => 0.5,
            crate::model_types::CostClass::High => 1.0,
            crate::model_types::CostClass::Unknown => 0.75,
        })
        .max(0.0)
}

fn latency_value(candidate: &ModelCandidate) -> f64 {
    candidate
        .metadata
        .health
        .p95_latency_ms
        .map(|value| value as f64)
        .unwrap_or_else(|| match candidate.metadata.attributes.latency_class {
            crate::model_types::LatencyClass::Fast => 100.0,
            crate::model_types::LatencyClass::Normal => 1000.0,
            crate::model_types::LatencyClass::Slow => 5000.0,
            crate::model_types::LatencyClass::Unknown => 2000.0,
        })
        .max(0.0)
}

fn risk_value(candidate: &ModelCandidate) -> f64 {
    candidate
        .metadata
        .health
        .error_rate_5m
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

fn quality_penalty(candidate: &ModelCandidate) -> f64 {
    1.0 - candidate
        .metadata
        .attributes
        .quality_score
        .unwrap_or(0.5)
        .clamp(0.0, 1.0)
}

fn preference_penalty(candidate: &ModelCandidate) -> f64 {
    let weight = (candidate.exact_model_weight * candidate.provider_weight).max(0.000_001);
    1.0 / weight
}

fn value_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in values {
        min = min.min(value);
        max = max.max(value);
    }
    if !min.is_finite() || !max.is_finite() {
        (0.0, 0.0)
    } else {
        (min, max)
    }
}

fn normalize(value: f64, range: (f64, f64)) -> f64 {
    if (range.1 - range.0).abs() < f64::EPSILON {
        return 0.0;
    }
    ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0)
}

fn ranked_trace(
    candidates: &[ModelCandidate],
    selected: Option<&ModelCandidate>,
    scored: Option<&[ScoredCandidate]>,
) -> Vec<RankedCandidateTrace> {
    candidates
        .iter()
        .map(|candidate| {
            let final_score = scored.and_then(|items| {
                items
                    .iter()
                    .find(|item| item.candidate.exact_model == candidate.exact_model)
                    .map(|item| item.score)
            });
            let score_inputs = scored
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.candidate.exact_model == candidate.exact_model)
                        .map(|item| item.inputs.clone())
                })
                .unwrap_or_default();
            RankedCandidateTrace {
                exact_model: candidate.exact_model.clone(),
                provider_instance_name: candidate.provider_instance_name.clone(),
                pricing_snapshot: RoutePricingSnapshot::from_candidate(candidate),
                priority_path: candidate.priority_path.clone(),
                exact_model_weight: candidate.exact_model_weight,
                provider_weight: candidate.provider_weight,
                preference_score_inputs: PreferenceScoreInputs::from_weights(
                    candidate.exact_model_weight,
                    candidate.provider_weight,
                ),
                score_inputs,
                final_score,
                selected: selected
                    .map(|item| item.exact_model == candidate.exact_model)
                    .unwrap_or(false),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_types::{
        ApiType, CostClass, LatencyClass, ModelAttributes, ModelCandidate, ModelCapabilities,
        ModelHealth, ModelMetadata, ModelPricing, ProviderType, QuotaState, SchedulerProfileConfig,
        SchedulerProfileWeights,
    };

    fn candidate(
        provider: &str,
        exact: &str,
        cost: f64,
        latency: u64,
        quality: f64,
        provider_type: ProviderType,
    ) -> ModelCandidate {
        let metadata = ModelMetadata {
            provider_model_id: exact.to_string(),
            exact_model: format!("{}@{}", exact, provider),
            model_driver: provider.to_string(),
            origin_model_id: None,
            provider_actual_model_id: None,
            provider_options: None,
            parameter_scale: None,
            api_types: vec![ApiType::Llm],
            logical_mounts: vec!["llm.plan".to_string()],
            capabilities: ModelCapabilities::default(),
            attributes: ModelAttributes {
                provider_type,
                quality_score: Some(quality),
                latency_class: LatencyClass::Normal,
                cost_class: CostClass::Medium,
                ..Default::default()
            },
            pricing: ModelPricing {
                estimated_cost: Some(cost),
                ..Default::default()
            },
            health: ModelHealth {
                p95_latency_ms: Some(latency),
                quota_state: QuotaState::Normal,
                ..Default::default()
            },
        };
        ModelCandidate::from_metadata(metadata, ApiType::Llm).unwrap()
    }

    fn candidates() -> Vec<ModelCandidate> {
        vec![
            candidate(
                "cheap",
                "model-cheap",
                0.001,
                2500,
                0.60,
                ProviderType::CloudApi,
            ),
            candidate(
                "fast",
                "model-fast",
                0.02,
                200,
                0.70,
                ProviderType::CloudApi,
            ),
            candidate(
                "quality",
                "model-quality",
                0.05,
                2000,
                0.98,
                ProviderType::CloudApi,
            ),
            candidate(
                "local",
                "model-local",
                0.0,
                800,
                0.50,
                ProviderType::LocalInference,
            ),
        ]
    }

    fn selected(profile: SchedulerProfile) -> String {
        let policy = RoutePolicy {
            profile,
            ..Default::default()
        };
        ModelScheduler
            .schedule(&candidates(), &policy)
            .unwrap()
            .selected
            .provider_instance_name
    }

    #[test]
    fn cost_first_selects_lowest_cost() {
        assert_eq!(selected(SchedulerProfile::CostFirst), "local");
    }

    #[test]
    fn latency_first_selects_fastest() {
        assert_eq!(selected(SchedulerProfile::LatencyFirst), "fast");
    }

    #[test]
    fn quality_first_selects_highest_quality() {
        assert_eq!(selected(SchedulerProfile::QualityFirst), "quality");
    }

    #[test]
    fn local_first_selects_local_candidate() {
        assert_eq!(selected(SchedulerProfile::LocalFirst), "local");
    }

    #[test]
    fn profile_weights_can_be_overridden_by_policy() {
        let policy = RoutePolicy {
            profile: SchedulerProfile::CostFirst,
            scheduler_profiles: Some(SchedulerProfileConfig {
                cost_first: Some(SchedulerProfileWeights {
                    cost: 0.0,
                    latency: 0.0,
                    reliability: 0.0,
                    quality: 1.0,
                    preference: 0.0,
                    cache: 0.0,
                    local: 0.0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = ModelScheduler.schedule(&candidates(), &policy).unwrap();

        assert_eq!(result.selected.provider_instance_name, "quality");
    }

    #[test]
    fn provider_weight_participates_in_preference_scoring() {
        let mut items = vec![
            candidate(
                "primary",
                "same-model",
                0.01,
                1000,
                0.90,
                ProviderType::CloudApi,
            ),
            candidate(
                "backup",
                "same-model",
                0.01,
                1000,
                0.90,
                ProviderType::CloudApi,
            ),
        ];
        items[0].provider_weight = 0.3;
        items[1].provider_weight = 2.0;
        let policy = RoutePolicy {
            profile: SchedulerProfile::Balanced,
            scheduler_profiles: Some(SchedulerProfileConfig {
                balanced: Some(SchedulerProfileWeights {
                    cost: 0.0,
                    latency: 0.0,
                    reliability: 0.0,
                    quality: 0.0,
                    preference: 1.0,
                    cache: 0.0,
                    local: 0.0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = ModelScheduler.schedule(&items, &policy).unwrap();

        assert_eq!(result.selected.provider_instance_name, "backup");
        assert_eq!(result.ranked_candidates[1].provider_weight, 2.0);
        assert_eq!(
            result.ranked_candidates[0]
                .preference_score_inputs
                .provider_weight,
            0.3
        );
        assert_eq!(
            result.ranked_candidates[0]
                .preference_score_inputs
                .provider_weight_effect,
            "downweighted"
        );
        assert_eq!(
            result.ranked_candidates[1]
                .preference_score_inputs
                .provider_weight_effect,
            "upweighted"
        );
        assert_eq!(
            result.ranked_candidates[1]
                .preference_score_inputs
                .combined_weight,
            2.0
        );
    }
}
