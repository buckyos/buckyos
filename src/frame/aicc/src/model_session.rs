use crate::model_types::{
    is_valid_provider_instance_name, FallbackMode, FallbackRule, LockedValue, LogicalItems,
    ModelDisable, ModelItem, ModelItemPatch, OverlayMergeMode, PolicyConfig, RouteError,
    RouteErrorCode, SessionOverlayTrace,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogicalNode {
    #[serde(default)]
    pub children: BTreeMap<String, LogicalNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub items: Option<LogicalItems>,
    #[serde(default)]
    pub item_overrides: Option<BTreeMap<String, ModelItemPatch>>,
    #[serde(default)]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub disable_line: Option<ModelDisable>,
    #[serde(default)]
    pub fallback: Option<FallbackRule>,
    #[serde(default)]
    pub policy: Option<PolicyConfig>,
    #[serde(default)]
    pub route_policy_override: Option<PolicyConfig>,
}

impl LogicalNode {
    pub fn effective_items(
        &self,
        inherited: Option<&LogicalItems>,
    ) -> Result<LogicalItems, RouteError> {
        if self.items.is_some() && self.item_overrides.is_some() {
            return Err(RouteError::new(
                RouteErrorCode::SessionConfigInvalid,
                "items and item_overrides cannot appear on the same logical node",
            ));
        }

        let mut items = self
            .items
            .clone()
            .or_else(|| inherited.cloned())
            .unwrap_or_default();

        if let Some(overrides) = self.item_overrides.as_ref() {
            for (name, patch) in overrides.iter() {
                if let Some(base) = items.get(name).cloned() {
                    items.insert(name.clone(), patch.apply_to(&base));
                } else {
                    items.insert(
                        name.clone(),
                        ModelItem::new(
                            patch.target.clone().unwrap_or_else(|| name.clone()),
                            patch.weight.unwrap_or(1.0),
                        ),
                    );
                }
            }
        }

        Ok(items)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub logical_tree: BTreeMap<String, LogicalNode>,
    #[serde(default)]
    pub logical_profile: Option<SessionLogicalProfile>,
    #[serde(default)]
    pub logical_profiles: BTreeMap<String, SessionLogicalProfile>,
    #[serde(default)]
    pub active_logical_profile: Option<String>,
    #[serde(default)]
    pub global_exact_model_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub provider_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionLogicalProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub overlays: Vec<LogicalTreeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<PolicyConfig>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogicalTreeOverlay {
    pub path: String,
    #[serde(default)]
    pub merge_mode: OverlayMergeMode,
    #[serde(default)]
    pub items: LogicalItems,
    #[serde(default)]
    pub item_overrides: BTreeMap<String, ModelItemPatch>,
    #[serde(default)]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<ModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<PolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EffectiveSessionConfig {
    pub config: SessionConfig,
    pub overlay_trace: Vec<SessionOverlayTrace>,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<(), RouteError> {
        for weight in self.global_exact_model_weights.values() {
            validate_weight(*weight)?;
        }
        for (provider_instance_name, weight) in self.provider_weights.iter() {
            if !is_valid_provider_instance_name(provider_instance_name) {
                return Err(RouteError::new(
                    RouteErrorCode::SessionConfigInvalid,
                    "provider weight key must be a valid provider instance name",
                ));
            }
            validate_weight(*weight)?;
        }
        validate_policy_values(&self.policy)?;
        for node in self.logical_tree.values() {
            validate_node(node)?;
        }
        if let Some(profile) = self.logical_profile.as_ref() {
            validate_profile(profile)?;
        }
        for profile in self.logical_profiles.values() {
            validate_profile(profile)?;
        }
        Ok(())
    }

    pub fn node(&self, path: &str) -> Option<&LogicalNode> {
        let mut parts = path.split('.');
        let first = parts.next()?;
        let mut node = self.logical_tree.get(first)?;
        for part in parts {
            node = node.children.get(part)?;
        }
        Some(node)
    }

    pub fn node_exact_weight(&self, path: &str, exact_model: &str) -> f64 {
        self.node(path)
            .and_then(|node| node.exact_model_weights.get(exact_model).copied())
            .or_else(|| self.global_exact_model_weights.get(exact_model).copied())
            .unwrap_or(1.0)
    }

    pub fn provider_weight(&self, provider_instance_name: &str) -> f64 {
        self.provider_weights
            .get(provider_instance_name)
            .copied()
            .unwrap_or(1.0)
    }
}

pub fn build_effective_session_config(
    config: &SessionConfig,
) -> Result<EffectiveSessionConfig, RouteError> {
    config.validate()?;
    let mut effective = config.clone();
    let mut trace = Vec::new();

    if let Some(profile_name) = config.active_logical_profile.as_deref() {
        let profile = config.logical_profiles.get(profile_name).ok_or_else(|| {
            RouteError::new(
                RouteErrorCode::SessionConfigInvalid,
                format!("active logical profile not found: {}", profile_name),
            )
        })?;
        apply_session_profile(&mut effective, profile, "session", &mut trace)?;
    }
    if let Some(profile) = config.logical_profile.as_ref() {
        apply_session_profile(&mut effective, profile, "session", &mut trace)?;
    }

    effective.logical_profile = None;
    effective.logical_profiles.clear();
    effective.active_logical_profile = None;
    effective.validate()?;
    Ok(EffectiveSessionConfig {
        config: effective,
        overlay_trace: trace,
    })
}

pub fn merge_session_config(
    parent: &SessionConfig,
    child: &SessionConfig,
) -> Result<SessionConfig, RouteError> {
    reject_locked_policy_patch(&parent.policy, &child.policy)?;
    let mut merged = parent.clone();
    merged.inherit = child.inherit.clone().or_else(|| parent.inherit.clone());
    merge_policy_config(&mut merged.policy, &child.policy);
    merged
        .global_exact_model_weights
        .extend(child.global_exact_model_weights.clone());
    merged
        .provider_weights
        .extend(child.provider_weights.clone());
    merged
        .logical_profiles
        .extend(child.logical_profiles.clone());
    if child.logical_profile.is_some() {
        merged.logical_profile = child.logical_profile.clone();
    }
    if child.active_logical_profile.is_some() {
        merged.active_logical_profile = child.active_logical_profile.clone();
    }
    merge_tree(&mut merged.logical_tree, &child.logical_tree)?;
    if child.ttl_seconds.is_some() {
        merged.ttl_seconds = child.ttl_seconds;
    }
    merged.revision = child.revision.clone().or_else(|| parent.revision.clone());
    merged.validate()?;
    Ok(merged)
}

fn merge_tree(
    base: &mut BTreeMap<String, LogicalNode>,
    patch: &BTreeMap<String, LogicalNode>,
) -> Result<(), RouteError> {
    for (name, patch_node) in patch.iter() {
        if let Some(base_node) = base.get_mut(name) {
            merge_node(base_node, patch_node)?;
        } else {
            base.insert(name.clone(), patch_node.clone());
        }
    }
    Ok(())
}

fn merge_node(base: &mut LogicalNode, patch: &LogicalNode) -> Result<(), RouteError> {
    if patch.items.is_some() && patch.item_overrides.is_some() {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "items and item_overrides cannot appear on the same logical node",
        ));
    }
    if let Some(items) = patch.items.as_ref() {
        base.items = Some(items.clone());
        base.item_overrides = None;
    }
    if let Some(overrides) = patch.item_overrides.as_ref() {
        let current = base.effective_items(None)?;
        let mut patched = current;
        for (name, item_patch) in overrides.iter() {
            if let Some(existing) = patched.get(name).cloned() {
                patched.insert(name.clone(), item_patch.apply_to(&existing));
            } else {
                patched.insert(
                    name.clone(),
                    ModelItem::new(
                        item_patch.target.clone().unwrap_or_else(|| name.clone()),
                        item_patch.weight.unwrap_or(1.0),
                    ),
                );
            }
        }
        base.items = Some(patched);
        base.item_overrides = None;
    }
    base.exact_model_weights
        .extend(patch.exact_model_weights.clone());
    if patch.fallback.is_some() {
        base.fallback = patch.fallback.clone();
    }
    if patch.disable_line.is_some() {
        base.disable_line = patch.disable_line.clone();
    }
    if let Some(policy) = patch.policy.as_ref() {
        let current = base.policy.get_or_insert_with(PolicyConfig::default);
        reject_locked_policy_patch(current, policy)?;
        merge_policy_config(current, policy);
    }
    if let Some(policy) = patch.route_policy_override.as_ref() {
        let current = base
            .route_policy_override
            .get_or_insert_with(PolicyConfig::default);
        reject_locked_policy_patch(current, policy)?;
        merge_policy_config(current, policy);
    }
    if patch.source.is_some() {
        base.source = patch.source.clone();
    }
    merge_tree(&mut base.children, &patch.children)?;
    Ok(())
}

fn apply_session_profile(
    effective: &mut SessionConfig,
    profile: &SessionLogicalProfile,
    scope: &str,
    trace: &mut Vec<SessionOverlayTrace>,
) -> Result<(), RouteError> {
    if let Some(policy) = profile.route_policy_override.as_ref() {
        reject_locked_policy_patch(&effective.policy, policy)?;
        merge_policy_config(&mut effective.policy, policy);
    }
    for overlay in profile.overlays.iter() {
        apply_logical_tree_overlay(effective, overlay)?;
        trace.push(SessionOverlayTrace {
            logical_profile_scope: scope.to_string(),
            overlay_path: overlay.path.clone(),
            merge_mode: overlay.merge_mode.clone(),
            selected_from_overlay: false,
        });
    }
    Ok(())
}

fn apply_logical_tree_overlay(
    effective: &mut SessionConfig,
    overlay: &LogicalTreeOverlay,
) -> Result<(), RouteError> {
    validate_overlay(overlay)?;
    let node = overlay_patch_node(overlay);
    let tree = tree_patch_for_path(overlay.path.as_str(), node)?;
    merge_tree(&mut effective.logical_tree, &tree)
}

fn overlay_patch_node(overlay: &LogicalTreeOverlay) -> LogicalNode {
    let source = overlay
        .source
        .clone()
        .or_else(|| Some("session_overlay".to_string()));
    let mut node = LogicalNode {
        source,
        exact_model_weights: overlay.exact_model_weights.clone(),
        disable_line: overlay.disable_line.clone(),
        fallback: overlay.fallback.clone(),
        route_policy_override: overlay.route_policy_override.clone(),
        ..Default::default()
    };

    match overlay.merge_mode {
        OverlayMergeMode::Replace => {
            node.items = Some(replace_items_for_overlay(overlay));
            if node.fallback.is_none() {
                node.fallback = Some(FallbackRule {
                    mode: FallbackMode::Disabled,
                    target: None,
                });
            }
        }
        OverlayMergeMode::Inherit => {
            let mut overrides = overlay.item_overrides.clone();
            for (name, item) in overlay.items.iter() {
                overrides.insert(
                    name.clone(),
                    ModelItemPatch {
                        target: Some(item.target.clone()),
                        weight: Some(item.weight),
                    },
                );
            }
            if !overrides.is_empty() {
                node.item_overrides = Some(overrides);
            }
        }
    }
    node
}

fn replace_items_for_overlay(overlay: &LogicalTreeOverlay) -> LogicalItems {
    let mut items = overlay.items.clone();
    for (name, patch) in overlay.item_overrides.iter() {
        items.insert(
            name.clone(),
            ModelItem::new(
                patch.target.clone().unwrap_or_else(|| name.clone()),
                patch.weight.unwrap_or(1.0),
            ),
        );
    }
    items
}

fn tree_patch_for_path(
    path: &str,
    node: LogicalNode,
) -> Result<BTreeMap<String, LogicalNode>, RouteError> {
    let mut parts = path.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| part.trim().is_empty()) {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "overlay path must be a non-empty logical path",
        ));
    }
    let leaf = parts.pop().expect("path has at least one part");
    let mut current = LogicalNode {
        children: [(leaf.to_string(), node)].into_iter().collect(),
        ..Default::default()
    };
    for part in parts.into_iter().rev() {
        current = LogicalNode {
            children: [(part.to_string(), current)].into_iter().collect(),
            ..Default::default()
        };
    }
    Ok(current.children)
}

fn merge_policy_config(base: &mut PolicyConfig, patch: &PolicyConfig) {
    if patch.profile.is_some() {
        base.profile = patch.profile.clone();
    }
    if patch.scheduler_profiles.is_some() {
        base.scheduler_profiles = patch.scheduler_profiles.clone();
    }
    if patch.local_only.is_some() {
        base.local_only = patch.local_only.clone();
    }
    if patch.allow_fallback.is_some() {
        base.allow_fallback = patch.allow_fallback.clone();
    }
    if patch.allow_exact_model_fallback.is_some() {
        base.allow_exact_model_fallback = patch.allow_exact_model_fallback.clone();
    }
    if patch.runtime_failover.is_some() {
        base.runtime_failover = patch.runtime_failover.clone();
    }
    if patch.explain.is_some() {
        base.explain = patch.explain.clone();
    }
    if patch.blocked_provider_instances.is_some() {
        base.blocked_provider_instances = patch.blocked_provider_instances.clone();
    }
    if patch.allowed_provider_instances.is_some() {
        base.allowed_provider_instances = patch.allowed_provider_instances.clone();
    }
    if patch.max_estimated_cost_usd.is_some() {
        base.max_estimated_cost_usd = patch.max_estimated_cost_usd.clone();
    }
}

fn reject_locked_policy_patch(
    parent: &PolicyConfig,
    patch: &PolicyConfig,
) -> Result<(), RouteError> {
    macro_rules! check_locked {
        ($field:ident) => {
            if parent
                .$field
                .as_ref()
                .map(|value| value.locked)
                .unwrap_or(false)
                && patch.$field.is_some()
            {
                return Err(RouteError::new(
                    RouteErrorCode::PolicyLocked,
                    concat!("policy field is locked: ", stringify!($field)),
                ));
            }
        };
    }
    check_locked!(profile);
    check_locked!(scheduler_profiles);
    check_locked!(local_only);
    check_locked!(allow_fallback);
    check_locked!(allow_exact_model_fallback);
    check_locked!(runtime_failover);
    check_locked!(explain);
    check_locked!(blocked_provider_instances);
    check_locked!(allowed_provider_instances);
    check_locked!(max_estimated_cost_usd);
    Ok(())
}

fn validate_node(node: &LogicalNode) -> Result<(), RouteError> {
    if node.items.is_some() && node.item_overrides.is_some() {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "items and item_overrides cannot appear on the same logical node",
        ));
    }
    if let Some(items) = node.items.as_ref() {
        for item in items.values() {
            validate_weight(item.weight)?;
        }
    }
    if let Some(overrides) = node.item_overrides.as_ref() {
        for patch in overrides.values() {
            if let Some(weight) = patch.weight {
                validate_weight(weight)?;
            }
        }
    }
    for weight in node.exact_model_weights.values() {
        validate_weight(*weight)?;
    }
    if let Some(policy) = node.policy.as_ref() {
        validate_policy_values(policy)?;
    }
    if let Some(policy) = node.route_policy_override.as_ref() {
        validate_policy_values(policy)?;
    }
    for child in node.children.values() {
        validate_node(child)?;
    }
    Ok(())
}

fn validate_profile(profile: &SessionLogicalProfile) -> Result<(), RouteError> {
    if let Some(policy) = profile.route_policy_override.as_ref() {
        validate_policy_values(policy)?;
    }
    for overlay in profile.overlays.iter() {
        validate_overlay(overlay)?;
    }
    Ok(())
}

fn validate_overlay(overlay: &LogicalTreeOverlay) -> Result<(), RouteError> {
    if overlay.path.trim().is_empty() || overlay.path.split('.').any(|part| part.trim().is_empty())
    {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "overlay path must be a non-empty logical path",
        ));
    }
    for item in overlay.items.values() {
        validate_weight(item.weight)?;
    }
    for patch in overlay.item_overrides.values() {
        if let Some(weight) = patch.weight {
            validate_weight(weight)?;
        }
    }
    for weight in overlay.exact_model_weights.values() {
        validate_weight(*weight)?;
    }
    if let Some(policy) = overlay.route_policy_override.as_ref() {
        validate_policy_values(policy)?;
    }
    Ok(())
}

fn validate_policy_values(policy: &PolicyConfig) -> Result<(), RouteError> {
    if let Some(LockedValue { value, .. }) = policy.max_estimated_cost_usd.as_ref() {
        validate_weight(*value)?;
    }
    if let Some(LockedValue { value, .. }) = policy.scheduler_profiles.as_ref() {
        for weights in [
            value.cost_first.as_ref(),
            value.latency_first.as_ref(),
            value.quality_first.as_ref(),
            value.balanced.as_ref(),
            value.local_first.as_ref(),
            value.strict_local.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            weights.validate()?;
        }
    }
    Ok(())
}

fn validate_weight(weight: f64) -> Result<(), RouteError> {
    if !weight.is_finite() || weight < 0.0 {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "weight must be a non-negative finite number",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_types::{LockedValue, ModelItemPatch, SchedulerProfile};

    fn node_with_items(items: Vec<(&str, &str, f64)>) -> LogicalNode {
        LogicalNode {
            items: Some(
                items
                    .into_iter()
                    .map(|(name, target, weight)| {
                        (name.to_string(), ModelItem::new(target.to_string(), weight))
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn item_weight(config: &SessionConfig, path: &str, item_name: &str) -> f64 {
        config
            .node(path)
            .unwrap()
            .items
            .as_ref()
            .unwrap()
            .get(item_name)
            .unwrap()
            .weight
    }

    #[test]
    fn items_override_default_items() {
        let default_items: LogicalItems = [(
            "openai".to_string(),
            ModelItem::new("gpt-5.2@openai_primary", 1.0),
        )]
        .into_iter()
        .collect();
        let node = node_with_items(vec![("claude", "claude-sonnet@anthropic", 2.0)]);

        let effective = node.effective_items(Some(&default_items)).unwrap();
        assert_eq!(effective.len(), 1);
        assert_eq!(
            effective.get("claude").map(|item| item.target.as_str()),
            Some("claude-sonnet@anthropic")
        );
    }

    #[test]
    fn item_overrides_patch_inherited_items() {
        let mut parent = SessionConfig::default();
        parent.logical_tree.insert(
            "llm".to_string(),
            LogicalNode {
                children: [(
                    "gpt5".to_string(),
                    node_with_items(vec![("openai", "gpt-5.2@openai_primary", 1.0)]),
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        );
        let mut child = SessionConfig::default();
        child.logical_tree.insert(
            "llm".to_string(),
            LogicalNode {
                children: [(
                    "gpt5".to_string(),
                    LogicalNode {
                        item_overrides: Some(
                            [(
                                "openai".to_string(),
                                ModelItemPatch {
                                    target: None,
                                    weight: Some(3.0),
                                },
                            )]
                            .into_iter()
                            .collect(),
                        ),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        );

        let merged = merge_session_config(&parent, &child).unwrap();
        let item = merged
            .node("llm.gpt5")
            .unwrap()
            .items
            .as_ref()
            .unwrap()
            .get("openai")
            .unwrap();
        assert_eq!(item.target, "gpt-5.2@openai_primary");
        assert_eq!(item.weight, 3.0);
    }

    #[test]
    fn session_patch_reprioritizes_interactive_agent_without_touching_background_jobs() {
        let parent = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [(
                        "agent".to_string(),
                        LogicalNode {
                            children: [
                                (
                                    "chat".to_string(),
                                    node_with_items(vec![
                                        ("quality", "llm.gpt5", 5.0),
                                        ("fast_local", "llm.local", 1.0),
                                        ("budget", "llm.mini", 0.5),
                                    ]),
                                ),
                                (
                                    "background_summary".to_string(),
                                    node_with_items(vec![
                                        ("budget", "llm.mini", 6.0),
                                        ("quality", "llm.gpt5", 1.0),
                                        ("fast_local", "llm.local", 0.5),
                                    ]),
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
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let child = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [(
                        "agent".to_string(),
                        LogicalNode {
                            children: [(
                                "chat".to_string(),
                                LogicalNode {
                                    item_overrides: Some(
                                        [
                                            (
                                                "quality".to_string(),
                                                ModelItemPatch {
                                                    target: None,
                                                    weight: Some(2.0),
                                                },
                                            ),
                                            (
                                                "fast_local".to_string(),
                                                ModelItemPatch {
                                                    target: None,
                                                    weight: Some(8.0),
                                                },
                                            ),
                                            (
                                                "budget".to_string(),
                                                ModelItemPatch {
                                                    target: None,
                                                    weight: Some(0.0),
                                                },
                                            ),
                                        ]
                                        .into_iter()
                                        .collect(),
                                    ),
                                    ..Default::default()
                                },
                            )]
                            .into_iter()
                            .collect(),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let merged = merge_session_config(&parent, &child).unwrap();

        assert_eq!(item_weight(&merged, "llm.agent.chat", "fast_local"), 8.0);
        assert_eq!(item_weight(&merged, "llm.agent.chat", "quality"), 2.0);
        assert_eq!(item_weight(&merged, "llm.agent.chat", "budget"), 0.0);
        assert_eq!(
            merged
                .node("llm.agent.chat")
                .unwrap()
                .items
                .as_ref()
                .unwrap()
                .get("fast_local")
                .unwrap()
                .target,
            "llm.local"
        );
        assert_eq!(
            item_weight(&merged, "llm.agent.background_summary", "budget"),
            6.0
        );
        assert_eq!(
            item_weight(&merged, "llm.agent.background_summary", "quality"),
            1.0
        );
    }

    #[test]
    fn session_patch_can_bias_exact_provider_for_one_logical_path() {
        let parent = SessionConfig {
            global_exact_model_weights: [
                ("gpt-5.2@openai_primary".to_string(), 1.0),
                ("gpt-5.2@openai_backup".to_string(), 1.0),
                ("claude-sonnet@anthropic".to_string(), 1.0),
            ]
            .into_iter()
            .collect(),
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [
                        ("gpt5".to_string(), LogicalNode::default()),
                        ("planning".to_string(), LogicalNode::default()),
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
        let child = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [(
                        "gpt5".to_string(),
                        LogicalNode {
                            exact_model_weights: [
                                ("gpt-5.2@openai_primary".to_string(), 0.25),
                                ("gpt-5.2@openai_backup".to_string(), 5.0),
                            ]
                            .into_iter()
                            .collect(),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let merged = merge_session_config(&parent, &child).unwrap();

        assert_eq!(
            merged.node_exact_weight("llm.gpt5", "gpt-5.2@openai_backup"),
            5.0
        );
        assert_eq!(
            merged.node_exact_weight("llm.gpt5", "gpt-5.2@openai_primary"),
            0.25
        );
        assert_eq!(
            merged.node_exact_weight("llm.planning", "gpt-5.2@openai_backup"),
            1.0
        );
        assert_eq!(
            merged.node_exact_weight("llm.gpt5", "claude-sonnet@anthropic"),
            1.0
        );
    }

    #[test]
    fn session_patch_merges_provider_weights() {
        let parent = SessionConfig {
            provider_weights: [
                ("openai_primary".to_string(), 1.0),
                ("openai_backup".to_string(), 0.5),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let child = SessionConfig {
            provider_weights: [
                ("openai_backup".to_string(), 0.0),
                ("local_llama".to_string(), 2.0),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let merged = merge_session_config(&parent, &child).unwrap();

        assert_eq!(merged.provider_weight("openai_primary"), 1.0);
        assert_eq!(merged.provider_weight("openai_backup"), 0.0);
        assert_eq!(merged.provider_weight("local_llama"), 2.0);
        assert_eq!(merged.provider_weight("missing"), 1.0);
    }

    #[test]
    fn negative_weight_is_rejected() {
        let mut config = SessionConfig::default();
        config.logical_tree.insert(
            "llm".to_string(),
            node_with_items(vec![("bad", "llm.gpt5", -1.0)]),
        );

        let err = config.validate().unwrap_err();
        assert_eq!(err.code, RouteErrorCode::SessionConfigInvalid);
    }

    #[test]
    fn items_and_item_overrides_together_are_rejected() {
        let config = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    items: Some(BTreeMap::new()),
                    item_overrides: Some(BTreeMap::new()),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let err = config.validate().unwrap_err();
        assert_eq!(err.code, RouteErrorCode::SessionConfigInvalid);
    }

    #[test]
    fn policy_lock_rejects_lower_patch() {
        let parent = SessionConfig {
            policy: PolicyConfig {
                local_only: Some(LockedValue::locked(true)),
                ..Default::default()
            },
            ..Default::default()
        };
        let child = SessionConfig {
            policy: PolicyConfig {
                local_only: Some(LockedValue::new(false)),
                ..Default::default()
            },
            ..Default::default()
        };

        let err = merge_session_config(&parent, &child).unwrap_err();
        assert_eq!(err.code, RouteErrorCode::PolicyLocked);
    }

    #[test]
    fn policy_patch_can_change_unlocked_profile() {
        let parent = SessionConfig::default();
        let child = SessionConfig {
            policy: PolicyConfig {
                profile: Some(LockedValue::new(SchedulerProfile::QualityFirst)),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_session_config(&parent, &child).unwrap();
        assert_eq!(
            merged.policy.profile.unwrap().value,
            SchedulerProfile::QualityFirst
        );
    }

    #[test]
    fn logical_profile_inherit_overlay_maps_to_item_overrides() {
        let config = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                node_with_items(vec![
                    ("quality", "gpt-5.2@openai_primary", 2.0),
                    ("fast", "qwen3@local", 1.0),
                ]),
            )]
            .into_iter()
            .collect(),
            logical_profile: Some(SessionLogicalProfile {
                name: Some("interactive".to_string()),
                overlays: vec![LogicalTreeOverlay {
                    path: "llm".to_string(),
                    merge_mode: OverlayMergeMode::Inherit,
                    item_overrides: [(
                        "fast".to_string(),
                        ModelItemPatch {
                            target: None,
                            weight: Some(8.0),
                        },
                    )]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }],
                route_policy_override: None,
            }),
            ..Default::default()
        };

        let effective = build_effective_session_config(&config).unwrap();

        assert_eq!(item_weight(&effective.config, "llm", "fast"), 8.0);
        assert_eq!(item_weight(&effective.config, "llm", "quality"), 2.0);
        assert_eq!(effective.overlay_trace.len(), 1);
        assert_eq!(effective.overlay_trace[0].overlay_path, "llm");
        assert_eq!(
            effective.overlay_trace[0].merge_mode,
            OverlayMergeMode::Inherit
        );
    }

    #[test]
    fn logical_profile_replace_overlay_maps_to_only_items_and_disables_fallback() {
        let config = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                node_with_items(vec![
                    ("quality", "gpt-5.2@openai_primary", 2.0),
                    ("fast", "qwen3@local", 1.0),
                ]),
            )]
            .into_iter()
            .collect(),
            logical_profile: Some(SessionLogicalProfile {
                name: Some("only-fast".to_string()),
                overlays: vec![LogicalTreeOverlay {
                    path: "llm".to_string(),
                    merge_mode: OverlayMergeMode::Replace,
                    items: [("fast".to_string(), ModelItem::new("qwen3@local", 1.0))]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                }],
                route_policy_override: None,
            }),
            ..Default::default()
        };

        let effective = build_effective_session_config(&config).unwrap();
        let node = effective.config.node("llm").unwrap();
        let items = node.items.as_ref().unwrap();

        assert_eq!(items.len(), 1);
        assert!(items.contains_key("fast"));
        assert_eq!(
            node.fallback.as_ref().map(|rule| rule.mode.clone()),
            Some(FallbackMode::Disabled)
        );
    }
}
