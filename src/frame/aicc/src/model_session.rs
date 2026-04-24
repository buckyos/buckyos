use crate::model_types::{
    FallbackRule, LockedValue, LogicalItems, ModelItem, ModelItemPatch, PolicyConfig, RouteError,
    RouteErrorCode,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogicalNode {
    #[serde(default)]
    pub children: BTreeMap<String, LogicalNode>,
    #[serde(default)]
    pub items: Option<LogicalItems>,
    #[serde(default)]
    pub item_overrides: Option<BTreeMap<String, ModelItemPatch>>,
    #[serde(default)]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub fallback: Option<FallbackRule>,
    #[serde(default)]
    pub policy: Option<PolicyConfig>,
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
    pub global_exact_model_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<(), RouteError> {
        for weight in self.global_exact_model_weights.values() {
            validate_weight(*weight)?;
        }
        validate_policy_values(&self.policy)?;
        for node in self.logical_tree.values() {
            validate_node(node)?;
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
    if let Some(policy) = patch.policy.as_ref() {
        let current = base.policy.get_or_insert_with(PolicyConfig::default);
        reject_locked_policy_patch(current, policy)?;
        merge_policy_config(current, policy);
    }
    merge_tree(&mut base.children, &patch.children)?;
    Ok(())
}

fn merge_policy_config(base: &mut PolicyConfig, patch: &PolicyConfig) {
    if patch.profile.is_some() {
        base.profile = patch.profile.clone();
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
    for child in node.children.values() {
        validate_node(child)?;
    }
    Ok(())
}

fn validate_policy_values(policy: &PolicyConfig) -> Result<(), RouteError> {
    if let Some(LockedValue { value, .. }) = policy.max_estimated_cost_usd.as_ref() {
        validate_weight(*value)?;
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

#[derive(Clone, Debug)]
pub struct StoredSessionConfig {
    pub config: SessionConfig,
    pub revision: String,
}

#[derive(Clone, Debug)]
struct SessionState {
    config: SessionConfig,
    revision: String,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct SessionConfigStore {
    global: SessionConfig,
    ttl: Duration,
    sessions: Mutex<BTreeMap<String, SessionState>>,
    revision_counter: AtomicU64,
}

impl SessionConfigStore {
    pub fn new(global: SessionConfig, ttl: Duration) -> Result<Self, RouteError> {
        global.validate()?;
        Ok(Self {
            global,
            ttl,
            sessions: Mutex::new(BTreeMap::new()),
            revision_counter: AtomicU64::new(1),
        })
    }

    pub fn get_or_create(&self, session_id: &str) -> Result<StoredSessionConfig, RouteError> {
        let mut sessions = self.sessions.lock().expect("session store lock");
        self.drop_expired_locked(&mut sessions, None)?;
        if let Some(state) = sessions.get_mut(session_id) {
            state.expires_at = Instant::now() + self.ttl;
            return Ok(StoredSessionConfig {
                config: state.config.clone(),
                revision: state.revision.clone(),
            });
        }

        let revision = self.next_revision();
        let mut config = self.global.clone();
        config.revision = Some(revision.clone());
        sessions.insert(
            session_id.to_string(),
            SessionState {
                config: config.clone(),
                revision: revision.clone(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        Ok(StoredSessionConfig { config, revision })
    }

    pub fn replace(
        &self,
        session_id: &str,
        mut config: SessionConfig,
        expected_revision: Option<&str>,
    ) -> Result<StoredSessionConfig, RouteError> {
        config.validate()?;
        let mut sessions = self.sessions.lock().expect("session store lock");
        self.drop_expired_locked(&mut sessions, expected_revision)?;
        check_expected_revision(sessions.get(session_id), expected_revision)?;
        let revision = self.next_revision();
        config.revision = Some(revision.clone());
        sessions.insert(
            session_id.to_string(),
            SessionState {
                config: config.clone(),
                revision: revision.clone(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        Ok(StoredSessionConfig { config, revision })
    }

    pub fn patch(
        &self,
        session_id: &str,
        patch: SessionConfig,
        expected_revision: Option<&str>,
    ) -> Result<StoredSessionConfig, RouteError> {
        let mut sessions = self.sessions.lock().expect("session store lock");
        self.drop_expired_locked(&mut sessions, expected_revision)?;
        check_expected_revision(sessions.get(session_id), expected_revision)?;
        let current = sessions
            .get(session_id)
            .map(|state| state.config.clone())
            .unwrap_or_else(|| self.global.clone());
        let mut config = merge_session_config(&current, &patch)?;
        let revision = self.next_revision();
        config.revision = Some(revision.clone());
        sessions.insert(
            session_id.to_string(),
            SessionState {
                config: config.clone(),
                revision: revision.clone(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        Ok(StoredSessionConfig { config, revision })
    }

    fn next_revision(&self) -> String {
        let value = self.revision_counter.fetch_add(1, Ordering::Relaxed);
        format!("session-rev-{}", value)
    }

    fn drop_expired_locked(
        &self,
        sessions: &mut BTreeMap<String, SessionState>,
        expected_revision: Option<&str>,
    ) -> Result<(), RouteError> {
        let now = Instant::now();
        let mut expired_expected = false;
        sessions.retain(|_, state| {
            let expired = state.expires_at <= now;
            if expired && expected_revision == Some(state.revision.as_str()) {
                expired_expected = true;
            }
            !expired
        });
        if expired_expected {
            return Err(RouteError::new(
                RouteErrorCode::SessionConfigExpired,
                "expected session config revision has expired",
            ));
        }
        Ok(())
    }
}

fn check_expected_revision(
    state: Option<&SessionState>,
    expected_revision: Option<&str>,
) -> Result<(), RouteError> {
    if let Some(expected) = expected_revision {
        let Some(state) = state else {
            return Err(RouteError::new(
                RouteErrorCode::SessionConfigExpired,
                "expected session config revision is no longer available",
            ));
        };
        if state.revision != expected {
            return Err(RouteError::new(
                RouteErrorCode::SessionConfigConflict,
                "session config revision conflict",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_types::{LockedValue, ModelItemPatch, SchedulerProfile};
    use std::thread;

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
    fn session_store_keeps_priority_patches_isolated_by_session_id() {
        let global = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
                LogicalNode {
                    children: [(
                        "chat".to_string(),
                        node_with_items(vec![
                            ("quality", "llm.gpt5", 5.0),
                            ("fast_local", "llm.local", 1.0),
                        ]),
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
        let store = SessionConfigStore::new(global, Duration::from_secs(30)).unwrap();
        let initial_interactive = store.get_or_create("interactive-agent").unwrap();
        let patch = SessionConfig {
            logical_tree: [(
                "llm".to_string(),
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
                                            weight: Some(1.0),
                                        },
                                    ),
                                    (
                                        "fast_local".to_string(),
                                        ModelItemPatch {
                                            target: None,
                                            weight: Some(10.0),
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
        };

        let interactive = store
            .patch(
                "interactive-agent",
                patch,
                Some(initial_interactive.revision.as_str()),
            )
            .unwrap();
        let background = store.get_or_create("background-worker").unwrap();

        assert_eq!(
            item_weight(&interactive.config, "llm.chat", "fast_local"),
            10.0
        );
        assert_eq!(item_weight(&interactive.config, "llm.chat", "quality"), 1.0);
        assert_eq!(item_weight(&background.config, "llm.chat", "quality"), 5.0);
        assert_eq!(
            item_weight(&background.config, "llm.chat", "fast_local"),
            1.0
        );
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
    fn revision_conflict_is_reported() {
        let store =
            SessionConfigStore::new(SessionConfig::default(), Duration::from_secs(30)).unwrap();
        let stored = store.get_or_create("s1").unwrap();
        let err = store
            .patch("s1", SessionConfig::default(), Some("wrong-rev"))
            .unwrap_err();

        assert_eq!(err.code, RouteErrorCode::SessionConfigConflict);
        assert_ne!(stored.revision, "wrong-rev");
    }

    #[test]
    fn expired_revision_is_reported() {
        let store =
            SessionConfigStore::new(SessionConfig::default(), Duration::from_millis(1)).unwrap();
        let stored = store.get_or_create("s1").unwrap();
        thread::sleep(Duration::from_millis(5));

        let err = store
            .patch(
                "s1",
                SessionConfig::default(),
                Some(stored.revision.as_str()),
            )
            .unwrap_err();
        assert_eq!(err.code, RouteErrorCode::SessionConfigExpired);
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
}
