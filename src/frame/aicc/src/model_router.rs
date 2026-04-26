use crate::model_registry::ModelRegistry;
use crate::model_session::SessionConfig;
use crate::model_types::{
    ApiType, ExactModelName, FallbackMode, FallbackRule, FallbackTraceItem, FilteredCandidateTrace,
    HealthStatus, ModelCandidate, ProviderType, RequestedModelType, RouteError, RouteErrorCode,
    RoutePolicy, RouteTrace,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

const MAX_FALLBACK_DEPTH: usize = 5;

#[derive(Clone, Debug)]
pub struct RouteRequest {
    pub request_id: String,
    pub session_id: Option<String>,
    pub api_type: ApiType,
    pub model: String,
    pub policy: RoutePolicy,
    pub session_config_revision: Option<String>,
    pub session_config_updated: bool,
}

#[derive(Clone, Debug)]
pub struct RouteResolution {
    pub candidates: Vec<ModelCandidate>,
    pub trace: RouteTrace,
}

pub struct ModelRouter<'a> {
    registry: &'a ModelRegistry,
    session_config: &'a SessionConfig,
}

impl<'a> ModelRouter<'a> {
    pub fn new(registry: &'a ModelRegistry, session_config: &'a SessionConfig) -> Self {
        Self {
            registry,
            session_config,
        }
    }

    pub fn resolve(&self, request: RouteRequest) -> Result<RouteResolution, RouteError> {
        let requested_model_type = if crate::model_types::is_exact_model_name(&request.model) {
            RequestedModelType::Exact
        } else {
            RequestedModelType::Logical
        };
        let mut trace = RouteTrace {
            request_id: request.request_id.clone(),
            session_id: request.session_id.clone(),
            session_config_revision: request.session_config_revision.clone(),
            session_config_updated: request.session_config_updated,
            api_type: request.api_type.clone(),
            requested_model: request.model.clone(),
            requested_model_type,
            resolved_logical_path: None,
            selected_exact_model: None,
            selected_provider_instance_name: None,
            candidate_count_before_filter: 0,
            candidate_count_after_filter: 0,
            filtered_candidates: Vec::new(),
            ranked_candidates: Vec::new(),
            fallback_applied: false,
            fallback_chain: Vec::new(),
            session_sticky_hit: false,
            scheduler_profile: request.policy.profile.clone(),
            runtime_failover_count: 0,
            user_summary: None,
            warnings: Vec::new(),
        };

        if crate::model_types::is_exact_model_name(&request.model) {
            return self.resolve_exact_request(request, &mut trace);
        }

        let candidates = self.resolve_logical_filtered(
            request.model.as_str(),
            &request.api_type,
            &request.policy,
            &mut trace,
        )?;
        if !candidates.is_empty() {
            return Ok(RouteResolution { candidates, trace });
        }

        let candidates = self.resolve_fallback(
            request.model.as_str(),
            &request.api_type,
            &request.policy,
            &mut trace,
        )?;
        if candidates.is_empty() {
            return Err(RouteError::new(
                RouteErrorCode::NoCandidate,
                "no candidate after fallback",
            ));
        }
        Ok(RouteResolution { candidates, trace })
    }

    fn resolve_exact_request(
        &self,
        request: RouteRequest,
        trace: &mut RouteTrace,
    ) -> Result<RouteResolution, RouteError> {
        ExactModelName::parse(request.model.as_str())?;
        let mut candidates = self.resolve_exact_raw(request.model.as_str(), &request.api_type);
        trace.candidate_count_before_filter = candidates.len();
        candidates = self.apply_hard_filters(candidates, &request.policy, trace);
        trace.candidate_count_after_filter = candidates.len();

        if !candidates.is_empty() {
            return Ok(RouteResolution {
                candidates,
                trace: trace.clone(),
            });
        }

        if !request.policy.allow_exact_model_fallback {
            return Err(RouteError::new(
                RouteErrorCode::ExactModelUnavailable,
                "exact model is unavailable and exact fallback is disabled",
            ));
        }

        let fallback = self.resolve_fallback(
            request.model.as_str(),
            &request.api_type,
            &request.policy,
            trace,
        )?;
        if fallback.is_empty() {
            return Err(RouteError::new(
                RouteErrorCode::NoCandidate,
                "exact model fallback produced no candidate",
            ));
        }
        Ok(RouteResolution {
            candidates: fallback,
            trace: trace.clone(),
        })
    }

    fn resolve_logical_filtered(
        &self,
        logical_path: &str,
        api_type: &ApiType,
        policy: &RoutePolicy,
        trace: &mut RouteTrace,
    ) -> Result<Vec<ModelCandidate>, RouteError> {
        let raw = self.resolve_logical_raw(logical_path, api_type)?;
        trace.candidate_count_before_filter += raw.len();
        let filtered = self.apply_hard_filters(raw, policy, trace);
        trace.candidate_count_after_filter += filtered.len();
        if !filtered.is_empty() {
            trace.resolved_logical_path = Some(logical_path.to_string());
        }
        Ok(select_highest_priority(filtered))
    }

    fn resolve_fallback(
        &self,
        from: &str,
        api_type: &ApiType,
        policy: &RoutePolicy,
        trace: &mut RouteTrace,
    ) -> Result<Vec<ModelCandidate>, RouteError> {
        if !policy.allow_fallback {
            return Ok(Vec::new());
        }

        let mut current = from.to_string();
        let mut visited = HashSet::<String>::new();
        for _ in 0..MAX_FALLBACK_DEPTH {
            if !visited.insert(current.clone()) {
                return Err(RouteError::new(
                    RouteErrorCode::FallbackLoop,
                    "fallback chain contains a loop",
                ));
            }

            let Some(rule) = self.fallback_rule(current.as_str(), policy) else {
                return Ok(Vec::new());
            };
            let Some(next) = fallback_target(current.as_str(), &rule) else {
                return Ok(Vec::new());
            };
            if !same_namespace(from, next.as_str(), api_type) {
                return Ok(Vec::new());
            }

            trace.fallback_applied = true;
            trace.fallback_chain.push(FallbackTraceItem {
                from: current.clone(),
                to: next.clone(),
                reason: format!("{:?}", rule.mode),
            });

            let raw = if crate::model_types::is_exact_model_name(next.as_str()) {
                self.resolve_exact_raw(next.as_str(), api_type)
            } else {
                self.resolve_logical_raw(next.as_str(), api_type)?
            };
            trace.candidate_count_before_filter += raw.len();
            let filtered = self.apply_hard_filters(raw, policy, trace);
            trace.candidate_count_after_filter += filtered.len();
            let selected = select_highest_priority(filtered);
            if !selected.is_empty() {
                trace.resolved_logical_path = Some(next.clone());
                return Ok(selected);
            }

            if crate::model_types::is_exact_model_name(next.as_str()) {
                return Ok(Vec::new());
            }
            current = next;
        }

        Err(RouteError::new(
            RouteErrorCode::FallbackLoop,
            "fallback chain exceeds max depth",
        ))
    }

    fn fallback_rule(&self, path: &str, policy: &RoutePolicy) -> Option<FallbackRule> {
        if let Some(node_rule) = self
            .session_config
            .node(path)
            .and_then(|node| node.fallback.clone())
        {
            return Some(node_rule);
        }
        if let Some(policy_rule) = policy.fallback.clone() {
            return Some(policy_rule);
        }
        if path.contains('.') {
            return Some(FallbackRule::parent());
        }
        None
    }

    fn resolve_exact_raw(&self, exact_model: &str, api_type: &ApiType) -> Vec<ModelCandidate> {
        self.registry
            .exact_candidate(exact_model, api_type)
            .into_iter()
            .map(|mut candidate| {
                candidate.priority_path = vec![1.0];
                candidate.route_paths = vec![exact_model.to_string()];
                candidate
            })
            .collect()
    }

    fn resolve_logical_raw(
        &self,
        logical_path: &str,
        api_type: &ApiType,
    ) -> Result<Vec<ModelCandidate>, RouteError> {
        let mut visited = HashSet::<String>::new();
        self.expand_logical(
            logical_path,
            logical_path,
            api_type,
            Vec::new(),
            &mut visited,
        )
    }

    fn expand_logical(
        &self,
        root_path: &str,
        logical_path: &str,
        api_type: &ApiType,
        priority_path: Vec<f64>,
        visited: &mut HashSet<String>,
    ) -> Result<Vec<ModelCandidate>, RouteError> {
        if !visited.insert(logical_path.to_string()) {
            return Err(RouteError::new(
                RouteErrorCode::LogicalTreeLoop,
                "logical tree item targets form a loop",
            ));
        }

        let default_items = self.registry.default_items_for_path(logical_path);
        let node = self.session_config.node(logical_path);
        if node.is_none() && default_items.is_empty() {
            visited.remove(logical_path);
            return Ok(Vec::new());
        }
        let items = if let Some(node) = node {
            node.effective_items(Some(&default_items))?
        } else {
            default_items
        };

        let mut candidates = Vec::new();
        for item in items.values() {
            if item.weight <= 0.0 {
                continue;
            }
            let mut next_priority = priority_path.clone();
            next_priority.push(item.weight);
            if crate::model_types::is_exact_model_name(item.target.as_str()) {
                for mut candidate in self.resolve_exact_raw(item.target.as_str(), api_type) {
                    candidate.resolved_logical_path = Some(root_path.to_string());
                    candidate.priority_path = next_priority.clone();
                    candidate.exact_model_weight = self
                        .session_config
                        .node_exact_weight(logical_path, candidate.exact_model.as_str());
                    candidate
                        .route_paths
                        .push(format!("{} -> {}", logical_path, item.target));
                    candidates.push(candidate);
                }
            } else {
                let nested = self.expand_logical(
                    root_path,
                    item.target.as_str(),
                    api_type,
                    next_priority,
                    visited,
                )?;
                candidates.extend(nested);
            }
        }

        visited.remove(logical_path);
        Ok(dedupe_candidates(candidates))
    }

    fn apply_hard_filters(
        &self,
        candidates: Vec<ModelCandidate>,
        policy: &RoutePolicy,
        trace: &mut RouteTrace,
    ) -> Vec<ModelCandidate> {
        candidates
            .into_iter()
            .filter_map(|candidate| {
                if !candidate.metadata.supports_api_type(&candidate.api_type) {
                    trace_drop(trace, &candidate, "api_type_mismatch");
                    return None;
                }
                if !candidate.metadata.is_available() {
                    let reason = if candidate.metadata.health.status == HealthStatus::Unavailable {
                        "model_unavailable"
                    } else {
                        "quota_exhausted"
                    };
                    trace_drop(trace, &candidate, reason);
                    return None;
                }
                if !candidate
                    .metadata
                    .supports_requirements(&policy.required_features)
                {
                    trace_drop(trace, &candidate, "feature_unsupported");
                    return None;
                }
                if policy.local_only
                    && candidate.metadata.attributes.provider_type != ProviderType::LocalInference
                {
                    trace_drop(trace, &candidate, "local_only");
                    return None;
                }
                if !policy.allowed_provider_instances.is_empty()
                    && !policy
                        .allowed_provider_instances
                        .iter()
                        .any(|item| item == &candidate.provider_instance_name)
                {
                    trace_drop(trace, &candidate, "provider_not_allowed");
                    return None;
                }
                if policy
                    .blocked_provider_instances
                    .iter()
                    .any(|item| item == &candidate.provider_instance_name)
                {
                    trace_drop(trace, &candidate, "provider_blocked");
                    return None;
                }
                if let Some(max_cost) = policy.max_estimated_cost_usd {
                    if candidate
                        .metadata
                        .pricing
                        .estimated_cost_usd
                        .map(|cost| cost > max_cost)
                        .unwrap_or(false)
                    {
                        trace_drop(trace, &candidate, "budget_exceeded");
                        return None;
                    }
                }
                if candidate.exact_model_weight <= 0.0 {
                    trace_drop(trace, &candidate, "exact_model_weight_zero");
                    return None;
                }
                Some(candidate)
            })
            .collect()
    }
}

fn trace_drop(trace: &mut RouteTrace, candidate: &ModelCandidate, reason: &str) {
    trace.filtered_candidates.push(FilteredCandidateTrace {
        exact_model: candidate.exact_model.clone(),
        provider_instance_name: candidate.provider_instance_name.clone(),
        reason: reason.to_string(),
    });
}

fn fallback_target(current: &str, rule: &FallbackRule) -> Option<String> {
    match rule.mode {
        FallbackMode::Strict | FallbackMode::Disabled => None,
        FallbackMode::Parent => current
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_string()),
        FallbackMode::TargetExact | FallbackMode::TargetLogical => rule.target.clone(),
    }
}

fn same_namespace(from: &str, to: &str, api_type: &ApiType) -> bool {
    if crate::model_types::is_exact_model_name(to) {
        return true;
    }
    from.split('.').next() == to.split('.').next()
        && to
            .split('.')
            .next()
            .map(|namespace| namespace == api_type.namespace())
            .unwrap_or(false)
}

fn dedupe_candidates(candidates: Vec<ModelCandidate>) -> Vec<ModelCandidate> {
    let mut deduped = BTreeMap::<(String, ApiType), ModelCandidate>::new();
    for candidate in candidates {
        let key = (candidate.exact_model.clone(), candidate.api_type.clone());
        match deduped.get_mut(&key) {
            Some(existing) => match compare_priority(&candidate, existing) {
                Ordering::Greater => {
                    deduped.insert(key, candidate);
                }
                Ordering::Equal => {
                    existing.route_paths.extend(candidate.route_paths.clone());
                }
                Ordering::Less => {}
            },
            None => {
                deduped.insert(key, candidate);
            }
        }
    }
    deduped.into_values().collect()
}

fn select_highest_priority(candidates: Vec<ModelCandidate>) -> Vec<ModelCandidate> {
    let best = candidates
        .iter()
        .max_by(|a, b| compare_priority(a, b))
        .cloned();
    let Some(best) = best else {
        return Vec::new();
    };
    candidates
        .into_iter()
        .filter(|candidate| compare_priority(candidate, &best) == Ordering::Equal)
        .collect()
}

fn compare_priority(a: &ModelCandidate, b: &ModelCandidate) -> Ordering {
    compare_f64_vec(&a.priority_path, &b.priority_path).then_with(|| {
        a.exact_model_weight
            .partial_cmp(&b.exact_model_weight)
            .unwrap_or(Ordering::Equal)
    })
}

fn compare_f64_vec(a: &[f64], b: &[f64]) -> Ordering {
    for (left, right) in a.iter().zip(b.iter()) {
        let ord = left.partial_cmp(right).unwrap_or(Ordering::Equal);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::ModelRegistry;
    use crate::model_session::{LogicalNode, SessionConfig};
    use crate::model_types::{
        CostClass, FallbackRule, ModelAttributes, ModelCapabilities, ModelHealth, ModelItem,
        ModelMetadata, ModelPricing, ProviderInventory, QuotaState, RoutePolicy,
    };

    fn metadata(
        provider: &str,
        model: &str,
        mount: &str,
        provider_type: ProviderType,
    ) -> ModelMetadata {
        ModelMetadata {
            provider_model_id: model.to_string(),
            exact_model: format!("{}@{}", model, provider),
            parameter_scale: None,
            api_types: vec![ApiType::LlmChat],
            logical_mounts: vec![mount.to_string()],
            capabilities: ModelCapabilities {
                streaming: true,
                tool_call: true,
                json_schema: true,
                vision: false,
                max_context_tokens: Some(128_000),
            },
            attributes: ModelAttributes {
                provider_type,
                quality_score: Some(0.9),
                cost_class: CostClass::Medium,
                ..Default::default()
            },
            pricing: ModelPricing {
                estimated_cost_usd: Some(0.01),
                ..Default::default()
            },
            health: ModelHealth {
                status: HealthStatus::Available,
                quota_state: QuotaState::Normal,
                ..Default::default()
            },
        }
    }

    fn registry() -> ModelRegistry {
        let mut registry = ModelRegistry::new();
        registry
            .apply_inventory(ProviderInventory {
                provider_instance_name: "openai_primary".to_string(),
                provider_type: ProviderType::CloudApi,
                provider_driver: "openai".to_string(),
                provider_origin: Default::default(),
                provider_type_trusted_source: Default::default(),
                provider_type_revision: None,
                version: None,
                inventory_revision: Some("r1".to_string()),
                models: vec![metadata(
                    "openai_primary",
                    "gpt-5.2",
                    "llm.gpt5",
                    ProviderType::CloudApi,
                )],
            })
            .unwrap();
        registry
            .apply_inventory(ProviderInventory {
                provider_instance_name: "anthropic".to_string(),
                provider_type: ProviderType::CloudApi,
                provider_driver: "claude".to_string(),
                provider_origin: Default::default(),
                provider_type_trusted_source: Default::default(),
                provider_type_revision: None,
                version: None,
                inventory_revision: Some("r1".to_string()),
                models: vec![metadata(
                    "anthropic",
                    "claude-sonnet",
                    "llm.claude",
                    ProviderType::CloudApi,
                )],
            })
            .unwrap();
        registry
            .apply_inventory(ProviderInventory {
                provider_instance_name: "local".to_string(),
                provider_type: ProviderType::LocalInference,
                provider_driver: "local".to_string(),
                provider_origin: Default::default(),
                provider_type_trusted_source: Default::default(),
                provider_type_revision: None,
                version: None,
                inventory_revision: Some("r1".to_string()),
                models: vec![metadata(
                    "local",
                    "qwen3",
                    "llm.local",
                    ProviderType::LocalInference,
                )],
            })
            .unwrap();
        registry
    }

    fn session_config() -> SessionConfig {
        SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    fallback: Some(FallbackRule::target_logical("llm.local")),
                    children: [
                        (
                            "plan".to_string(),
                            LogicalNode {
                                items: Some(
                                    [
                                        ("gpt-pro".to_string(), ModelItem::new("llm.gpt-pro", 3.0)),
                                        ("opus".to_string(), ModelItem::new("llm.opus", 3.0)),
                                    ]
                                    .into_iter()
                                    .collect(),
                                ),
                                ..Default::default()
                            },
                        ),
                        ("gpt5".to_string(), LogicalNode::default()),
                        ("claude".to_string(), LogicalNode::default()),
                        ("local".to_string(), LogicalNode::default()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }

    fn request(model: &str, policy: RoutePolicy) -> RouteRequest {
        RouteRequest {
            request_id: "req-1".to_string(),
            session_id: Some("s1".to_string()),
            api_type: ApiType::LlmChat,
            model: model.to_string(),
            policy,
            session_config_revision: Some("rev-1".to_string()),
            session_config_updated: false,
        }
    }

    #[test]
    fn logical_plan_routes_to_weighted_gpt_family() {
        let registry = registry();
        let config = session_config();
        let router = ModelRouter::new(&registry, &config);

        let resolved = router
            .resolve(request("llm.plan", RoutePolicy::default()))
            .unwrap();

        assert_eq!(resolved.candidates.len(), 1);
        assert_eq!(resolved.candidates[0].exact_model, "gpt-5.2@openai_primary");
    }

    #[test]
    fn logical_tree_loop_is_rejected() {
        let registry = registry();
        let config = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [
                        (
                            "a".to_string(),
                            LogicalNode {
                                items: Some(
                                    [("b".to_string(), ModelItem::new("llm.b", 1.0))]
                                        .into_iter()
                                        .collect(),
                                ),
                                ..Default::default()
                            },
                        ),
                        (
                            "b".to_string(),
                            LogicalNode {
                                items: Some(
                                    [("a".to_string(), ModelItem::new("llm.a", 1.0))]
                                        .into_iter()
                                        .collect(),
                                ),
                                ..Default::default()
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let router = ModelRouter::new(&registry, &config);

        let err = router
            .resolve(request("llm.a", RoutePolicy::default()))
            .unwrap_err();
        assert_eq!(err.code, RouteErrorCode::LogicalTreeLoop);
    }

    #[test]
    fn fallback_loop_is_rejected() {
        let registry = registry();
        let config = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [
                        (
                            "a".to_string(),
                            LogicalNode {
                                fallback: Some(FallbackRule::target_logical("llm.b")),
                                ..Default::default()
                            },
                        ),
                        (
                            "b".to_string(),
                            LogicalNode {
                                fallback: Some(FallbackRule::target_logical("llm.a")),
                                ..Default::default()
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let router = ModelRouter::new(&registry, &config);

        let err = router
            .resolve(request("llm.a", RoutePolicy::default()))
            .unwrap_err();
        assert_eq!(err.code, RouteErrorCode::FallbackLoop);
    }

    #[test]
    fn exact_model_does_not_fallback_by_default() {
        let registry = registry();
        let config = session_config();
        let router = ModelRouter::new(&registry, &config);

        let err = router
            .resolve(request("missing@openai_primary", RoutePolicy::default()))
            .unwrap_err();
        assert_eq!(err.code, RouteErrorCode::ExactModelUnavailable);
    }

    #[test]
    fn local_only_still_filters_after_fallback() {
        let registry = registry();
        let mut config = session_config();
        config.logical_tree.get_mut("llm").unwrap().fallback = None;
        config.logical_tree.get_mut("llm").unwrap().children.insert(
            "empty".to_string(),
            LogicalNode {
                fallback: Some(FallbackRule::target_logical("llm.gpt5")),
                ..Default::default()
            },
        );
        let router = ModelRouter::new(&registry, &config);
        let policy = RoutePolicy {
            local_only: true,
            ..Default::default()
        };

        let err = router.resolve(request("llm.empty", policy)).unwrap_err();
        assert_eq!(err.code, RouteErrorCode::NoCandidate);
    }
}
