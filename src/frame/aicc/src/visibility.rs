//! Provider / Model visibility by user group (product/ai_center/AI_Center_Quota_PRD.md §3.4).
//!
//! A user's allow items are the union of the complete items of their UserType
//! group and system user groups; what they see is that union intersected with
//! the actual inventory of each Provider. Nothing outside the inventory is
//! ever granted, and a missing policy grants nothing.

use std::collections::{BTreeMap, BTreeSet};

use buckyos_api::{
    UserType, VisibilityEntry, VisibilityGrant, VisibilitySubjectPolicy,
    VISIBILITY_SUBJECT_GROUP_PREFIX, VISIBILITY_SUBJECT_TYPE_PREFIX,
};
use serde_json::Value;

/// Exact models of every configured Provider instance (empty for a Provider
/// whose inventory has no model).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Inventory {
    pub providers: BTreeMap<String, BTreeSet<String>>,
    /// Configured but disabled Provider instances (with no inventory).
    pub disabled: BTreeSet<String>,
}

impl Inventory {
    pub(crate) fn from_runtime(snapshot: &crate::runtime::RuntimeSnapshot) -> Self {
        let mut providers = snapshot
            .settings
            .providers
            .iter()
            .map(|provider| (provider.provider_instance_name.clone(), BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for model in snapshot.models.model_views() {
            providers
                .entry(model.provider_instance_name.clone())
                .or_default()
                .insert(model.exact_model.clone());
        }
        let disabled = snapshot
            .settings
            .providers
            .iter()
            .filter(|provider| !provider.enabled)
            .map(|provider| provider.provider_instance_name.clone())
            .collect();
        Self {
            providers,
            disabled,
        }
    }

    /// From the Provider views (name, enabled) and the `models.list` value of
    /// an admin snapshot.
    pub(crate) fn from_views(
        provider_views: impl IntoIterator<Item = (String, bool)>,
        models: &Value,
    ) -> Self {
        let mut providers = BTreeMap::new();
        let mut disabled = BTreeSet::new();
        for (name, enabled) in provider_views {
            if !enabled {
                disabled.insert(name.clone());
            }
            providers.insert(name, BTreeSet::new());
        }
        for model in models
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(provider), Some(exact_model)) = (
                model.get("provider_instance_name").and_then(Value::as_str),
                model.get("exact_model").and_then(Value::as_str),
            ) {
                providers
                    .entry(provider.to_string())
                    .or_default()
                    .insert(exact_model.to_string());
            }
        }
        Self {
            providers,
            disabled,
        }
    }

    pub(crate) fn contains_model(&self, exact_model: &str) -> bool {
        self.providers
            .values()
            .any(|models| models.contains(exact_model))
    }
}

/// What a caller may see and use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// A system service or device acting for itself.
    Unrestricted,
    /// Visible Provider → permitted exact models (possibly none).
    Restricted(BTreeMap<String, BTreeSet<String>>),
}

impl Access {
    pub(crate) fn allows_model(&self, exact_model: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(providers) => providers
                .values()
                .any(|models| models.contains(exact_model)),
        }
    }

    pub(crate) fn allows_provider(&self, provider_instance_name: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(providers) => providers.contains_key(provider_instance_name),
        }
    }

    pub(crate) fn is_restricted(&self) -> bool {
        matches!(self, Self::Restricted(_))
    }
}

/// UserType group first, then the system groups that are still defined.
pub(crate) fn subjects_for(user_type: UserType, groups: &[String]) -> Vec<String> {
    let type_name = match user_type {
        UserType::Root | UserType::Admin => "admin",
        UserType::User => "user",
        UserType::Limited => "limited",
    };
    let mut subjects = vec![format!("{VISIBILITY_SUBJECT_TYPE_PREFIX}{type_name}")];
    subjects.extend(
        groups
            .iter()
            .map(|group| format!("{VISIBILITY_SUBJECT_GROUP_PREFIX}{group}")),
    );
    subjects
}

/// Every visible (Provider, exact model) with the subjects granting it; a
/// Provider granted without any current model appears with `None`.
pub(crate) fn grants(
    policy: &[VisibilitySubjectPolicy],
    subjects: &[String],
    inventory: &Inventory,
) -> BTreeMap<(String, Option<String>), BTreeSet<String>> {
    let mut result: BTreeMap<(String, Option<String>), BTreeSet<String>> = BTreeMap::new();
    for subject_policy in policy
        .iter()
        .filter(|policy| subjects.contains(&policy.subject))
    {
        let subject = &subject_policy.subject;
        for entry in &subject_policy.entries {
            let granted: Vec<(&String, Vec<&String>)> = match entry {
                VisibilityEntry::AllProviders => inventory
                    .providers
                    .iter()
                    .map(|(provider, models)| (provider, models.iter().collect()))
                    .collect(),
                VisibilityEntry::ProviderAllModels {
                    provider_instance_name,
                } => inventory
                    .providers
                    .get_key_value(provider_instance_name)
                    .map(|(provider, models)| vec![(provider, models.iter().collect())])
                    .unwrap_or_default(),
                VisibilityEntry::ProviderModels {
                    provider_instance_name,
                    exact_models,
                } => inventory
                    .providers
                    .get_key_value(provider_instance_name)
                    .map(|(provider, models)| {
                        vec![(
                            provider,
                            models
                                .iter()
                                .filter(|model| exact_models.contains(*model))
                                .collect(),
                        )]
                    })
                    .unwrap_or_default(),
            };
            for (provider, models) in granted {
                result
                    .entry((provider.clone(), None))
                    .or_default()
                    .insert(subject.clone());
                for model in models {
                    result
                        .entry((provider.clone(), Some(model.clone())))
                        .or_default()
                        .insert(subject.clone());
                }
            }
        }
    }
    result
}

pub(crate) fn access_for(
    policy: &[VisibilitySubjectPolicy],
    subjects: &[String],
    inventory: &Inventory,
) -> Access {
    let mut providers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((provider, model), _) in grants(policy, subjects, inventory) {
        let models = providers.entry(provider).or_default();
        if let Some(model) = model {
            models.insert(model);
        }
    }
    Access::Restricted(providers)
}

/// The grant list of a preview: per Provider, its models, or the Provider
/// alone when it has none to show.
pub(crate) fn grant_list(
    grants: &BTreeMap<(String, Option<String>), BTreeSet<String>>,
) -> Vec<VisibilityGrant> {
    let with_models = grants
        .keys()
        .filter(|(_, model)| model.is_some())
        .map(|(provider, _)| provider.clone())
        .collect::<BTreeSet<_>>();
    grants
        .iter()
        .filter(|((provider, model), _)| model.is_some() || !with_models.contains(provider))
        .map(|((provider, model), sources)| VisibilityGrant {
            provider_instance_name: provider.clone(),
            exact_model: model.clone(),
            sources: sources.iter().cloned().collect(),
        })
        .collect()
}

/// Exact models some subject of the policy can see.
pub(crate) fn exposed_models(policy: &[VisibilitySubjectPolicy], inventory: &Inventory) -> BTreeSet<String> {
    let subjects = policy
        .iter()
        .map(|policy| policy.subject.clone())
        .collect::<Vec<_>>();
    grants(policy, &subjects, inventory)
        .into_keys()
        .filter_map(|(_, model)| model)
        .collect()
}

pub(crate) fn validate_subject(subject: &str, defined_groups: &BTreeSet<String>) -> Result<(), &'static str> {
    if let Some(type_name) = subject.strip_prefix(VISIBILITY_SUBJECT_TYPE_PREFIX) {
        return match type_name {
            "admin" | "user" | "limited" => Ok(()),
            _ => Err("visibility subject must be type:admin, type:user or type:limited"),
        };
    }
    if let Some(group) = subject.strip_prefix(VISIBILITY_SUBJECT_GROUP_PREFIX) {
        return if defined_groups.contains(group) {
            Ok(())
        } else {
            Err("visibility subject names an undefined user group")
        };
    }
    Err("visibility subject must be type:<user_type> or group:<id>")
}

/// Each Provider appears in at most one item and models must be exact
/// models of that Provider.
pub(crate) fn validate_entries(entries: &[VisibilityEntry]) -> Result<(), &'static str> {
    let mut providers = BTreeSet::new();
    for entry in entries {
        match entry {
            VisibilityEntry::AllProviders => {}
            VisibilityEntry::ProviderAllModels {
                provider_instance_name,
            } => {
                if provider_instance_name.trim().is_empty() {
                    return Err("visibility item has no Provider instance");
                }
                if !providers.insert(provider_instance_name.as_str()) {
                    return Err("a Provider appears in more than one visibility item");
                }
            }
            VisibilityEntry::ProviderModels {
                provider_instance_name,
                exact_models,
            } => {
                if provider_instance_name.trim().is_empty() {
                    return Err("visibility item has no Provider instance");
                }
                if !providers.insert(provider_instance_name.as_str()) {
                    return Err("a Provider appears in more than one visibility item");
                }
                if exact_models.iter().any(|model| {
                    model.rsplit_once('@').map(|(_, provider)| provider)
                        != Some(provider_instance_name.as_str())
                }) {
                    return Err("an allowed model is not an exact model of its Provider");
                }
            }
        }
    }
    if entries
        .iter()
        .filter(|entry| matches!(entry, VisibilityEntry::AllProviders))
        .count()
        > 1
    {
        return Err("all_providers appears more than once");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Inventory {
        Inventory {
            providers: BTreeMap::from([
                (
                    "cloud-main".to_string(),
                    BTreeSet::from(["model-a@cloud-main".to_string(), "model-b@cloud-main".to_string()]),
                ),
                (
                    "other".to_string(),
                    BTreeSet::from(["model-c@other".to_string()]),
                ),
                ("empty".to_string(), BTreeSet::new()),
            ]),
            disabled: BTreeSet::new(),
        }
    }

    fn policy(subject: &str, entries: Vec<VisibilityEntry>) -> VisibilitySubjectPolicy {
        VisibilitySubjectPolicy {
            subject: subject.into(),
            entries,
            updated_by: "admin".into(),
            updated_at_ms: 0,
        }
    }

    #[test]
    fn limited_group_sees_only_the_chosen_model_of_its_provider() {
        let policies = vec![
            policy("type:user", vec![VisibilityEntry::AllProviders]),
            policy(
                "group:customer",
                vec![VisibilityEntry::ProviderModels {
                    provider_instance_name: "cloud-main".into(),
                    exact_models: vec!["model-a@cloud-main".into(), "model-x@cloud-main".into()],
                }],
            ),
        ];
        let access = access_for(&policies, &subjects_for(UserType::Limited, &["customer".into()]), &inventory());
        assert!(access.allows_model("model-a@cloud-main"));
        assert!(!access.allows_model("model-b@cloud-main"));
        assert!(!access.allows_model("model-x@cloud-main"));
        assert!(!access.allows_provider("other"));
        assert!(access.allows_provider("cloud-main"));
    }

    #[test]
    fn groups_union_and_missing_policy_grants_nothing() {
        let policies = vec![
            policy(
                "group:a",
                vec![VisibilityEntry::ProviderModels {
                    provider_instance_name: "cloud-main".into(),
                    exact_models: vec!["model-a@cloud-main".into()],
                }],
            ),
            policy(
                "group:b",
                vec![VisibilityEntry::ProviderAllModels {
                    provider_instance_name: "other".into(),
                }],
            ),
        ];
        let subjects = subjects_for(UserType::Limited, &["a".into(), "b".into()]);
        let access = access_for(&policies, &subjects, &inventory());
        assert!(access.allows_model("model-a@cloud-main"));
        assert!(access.allows_model("model-c@other"));
        assert!(!access.allows_model("model-b@cloud-main"));
        let nobody = access_for(&policies, &subjects_for(UserType::User, &[]), &inventory());
        assert_eq!(nobody, Access::Restricted(BTreeMap::new()));
    }

    #[test]
    fn all_providers_follows_the_inventory_and_lists_empty_providers() {
        let policies = vec![policy("type:admin", vec![VisibilityEntry::AllProviders])];
        let subjects = subjects_for(UserType::Root, &[]);
        let grants = grants(&policies, &subjects, &inventory());
        let list = grant_list(&grants);
        assert!(list.iter().any(|grant| grant.provider_instance_name == "empty" && grant.exact_model.is_none()));
        assert!(!list
            .iter()
            .any(|grant| grant.provider_instance_name == "cloud-main" && grant.exact_model.is_none()));
        assert_eq!(exposed_models(&policies, &inventory()).len(), 3);
    }

    #[test]
    fn entries_and_subjects_are_validated() {
        assert!(validate_entries(&[VisibilityEntry::ProviderModels {
            provider_instance_name: "cloud-main".into(),
            exact_models: vec!["model-c@other".into()],
        }])
        .is_err());
        assert!(validate_entries(&[
            VisibilityEntry::ProviderAllModels {
                provider_instance_name: "cloud-main".into(),
            },
            VisibilityEntry::ProviderModels {
                provider_instance_name: "cloud-main".into(),
                exact_models: vec![],
            },
        ])
        .is_err());
        let groups = BTreeSet::from(["customer".to_string()]);
        assert!(validate_subject("type:limited", &groups).is_ok());
        assert!(validate_subject("group:customer", &groups).is_ok());
        assert!(validate_subject("group:gone", &groups).is_err());
        assert!(validate_subject("type:root", &groups).is_err());
    }
}
