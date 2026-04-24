use crate::model_types::{
    ApiType, ExactModelName, LogicalItems, ModelCandidate, ModelItem, ProviderInventory,
    RouteError, RouteErrorCode,
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct ModelRegistry {
    inventories: HashMap<String, ProviderInventory>,
    exact_index: HashMap<(String, ApiType), ModelCandidate>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_inventory(&mut self, inventory: ProviderInventory) -> Result<(), RouteError> {
        validate_inventory(&inventory)?;
        self.inventories
            .insert(inventory.provider_instance_name.clone(), inventory);
        self.rebuild_index()
    }

    pub fn remove_inventory(&mut self, provider_instance_name: &str) -> Result<(), RouteError> {
        self.inventories.remove(provider_instance_name);
        self.rebuild_index()
    }

    pub fn inventory_revision(&self, provider_instance_name: &str) -> Option<&str> {
        self.inventories
            .get(provider_instance_name)
            .and_then(|inventory| inventory.inventory_revision.as_deref())
    }

    pub fn inventories(&self) -> impl Iterator<Item = &ProviderInventory> {
        self.inventories.values()
    }

    pub fn exact_candidate(&self, exact_model: &str, api_type: &ApiType) -> Option<ModelCandidate> {
        self.exact_index
            .get(&(exact_model.to_string(), api_type.clone()))
            .cloned()
    }

    pub fn default_items_for_path(&self, logical_path: &str) -> LogicalItems {
        default_items_from_inventories(self.inventories.values(), logical_path)
    }

    pub fn all_default_items(&self) -> BTreeMap<String, LogicalItems> {
        let mut mounts = BTreeMap::<String, LogicalItems>::new();
        for inventory in self.inventories.values() {
            for model in inventory.models.iter() {
                for mount in model.logical_mounts.iter() {
                    let item_name = default_item_name(model.exact_model.as_str());
                    mounts
                        .entry(mount.clone())
                        .or_default()
                        .insert(item_name, ModelItem::new(model.exact_model.clone(), 1.0));
                }
            }
        }
        mounts
    }

    fn rebuild_index(&mut self) -> Result<(), RouteError> {
        let mut next = HashMap::new();
        for inventory in self.inventories.values() {
            for model in inventory.models.iter() {
                for api_type in model.api_types.iter() {
                    let candidate = ModelCandidate::from_metadata(model.clone(), api_type.clone())?;
                    let key = (model.exact_model.clone(), api_type.clone());
                    if next.insert(key.clone(), candidate).is_some() {
                        return Err(RouteError::new(
                            RouteErrorCode::SessionConfigInvalid,
                            format!(
                                "duplicate exact model '{}' for api type '{:?}'",
                                key.0, key.1
                            ),
                        ));
                    }
                }
            }
        }
        self.exact_index = next;
        Ok(())
    }
}

pub fn default_items_from_inventories<'a>(
    inventories: impl Iterator<Item = &'a ProviderInventory>,
    logical_path: &str,
) -> LogicalItems {
    let mut items = BTreeMap::<String, ModelItem>::new();
    for inventory in inventories {
        for model in inventory.models.iter() {
            if model
                .logical_mounts
                .iter()
                .any(|mount| mount.as_str() == logical_path)
            {
                let item_name = default_item_name(model.exact_model.as_str());
                items.insert(item_name, ModelItem::new(model.exact_model.clone(), 1.0));
            }
        }
    }
    items
}

pub fn default_item_name(exact_model: &str) -> String {
    exact_model
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_inventory(inventory: &ProviderInventory) -> Result<(), RouteError> {
    if !crate::model_types::is_valid_provider_instance_name(&inventory.provider_instance_name) {
        return Err(RouteError::new(
            RouteErrorCode::InvalidModelName,
            "provider instance name is invalid",
        ));
    }

    let mut seen = HashSet::<String>::new();
    for model in inventory.models.iter() {
        let exact = ExactModelName::parse(model.exact_model.as_str())?;
        if exact.provider_instance_name != inventory.provider_instance_name {
            return Err(RouteError::new(
                RouteErrorCode::InvalidModelName,
                format!(
                    "exact model '{}' does not belong to provider '{}'",
                    model.exact_model, inventory.provider_instance_name
                ),
            ));
        }
        if exact.provider_model_id != model.provider_model_id {
            return Err(RouteError::new(
                RouteErrorCode::InvalidModelName,
                format!(
                    "exact model '{}' does not match provider model id '{}'",
                    model.exact_model, model.provider_model_id
                ),
            ));
        }
        if !seen.insert(model.exact_model.clone()) {
            return Err(RouteError::new(
                RouteErrorCode::SessionConfigInvalid,
                format!(
                    "duplicate exact model '{}' in provider '{}'",
                    model.exact_model, inventory.provider_instance_name
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_types::{
        CostClass, HealthStatus, ModelAttributes, ModelCapabilities, ModelHealth, ModelMetadata,
        ProviderType,
    };

    fn model(provider: &str, provider_model_id: &str, mount: &str) -> ModelMetadata {
        ModelMetadata {
            provider_model_id: provider_model_id.to_string(),
            exact_model: format!("{}@{}", provider_model_id, provider),
            parameter_scale: None,
            api_types: vec![ApiType::LlmChat],
            logical_mounts: vec![mount.to_string()],
            capabilities: ModelCapabilities::default(),
            attributes: ModelAttributes {
                provider_type: ProviderType::CloudApi,
                quality_score: Some(0.9),
                cost_class: CostClass::High,
                ..Default::default()
            },
            pricing: Default::default(),
            health: ModelHealth {
                status: HealthStatus::Available,
                ..Default::default()
            },
        }
    }

    fn inventory(provider: &str, revision: &str, models: Vec<ModelMetadata>) -> ProviderInventory {
        ProviderInventory {
            provider_instance_name: provider.to_string(),
            provider_type: ProviderType::CloudApi,
            version: None,
            inventory_revision: Some(revision.to_string()),
            models,
        }
    }

    #[test]
    fn same_logical_mount_keeps_multiple_providers() {
        let mut registry = ModelRegistry::new();
        registry
            .apply_inventory(inventory(
                "openai_primary",
                "r1",
                vec![model("openai_primary", "gpt-5.2", "llm.gpt5")],
            ))
            .unwrap();
        registry
            .apply_inventory(inventory(
                "openai_backup",
                "r1",
                vec![model("openai_backup", "gpt-5.2", "llm.gpt5")],
            ))
            .unwrap();

        let items = registry.default_items_for_path("llm.gpt5");
        assert_eq!(items.len(), 2);
        assert!(items
            .values()
            .any(|item| item.target == "gpt-5.2@openai_primary"));
        assert!(items
            .values()
            .any(|item| item.target == "gpt-5.2@openai_backup"));
    }

    #[test]
    fn duplicate_exact_model_in_same_provider_is_rejected() {
        let mut registry = ModelRegistry::new();
        let err = registry
            .apply_inventory(inventory(
                "openai_primary",
                "r1",
                vec![
                    model("openai_primary", "gpt-5.2", "llm.gpt5"),
                    model("openai_primary", "gpt-5.2", "llm.plan"),
                ],
            ))
            .unwrap_err();

        assert_eq!(err.code, RouteErrorCode::SessionConfigInvalid);
    }

    #[test]
    fn inventory_revision_replaces_provider_snapshot() {
        let mut registry = ModelRegistry::new();
        registry
            .apply_inventory(inventory(
                "openai_primary",
                "r1",
                vec![model("openai_primary", "gpt-5.1", "llm.gpt5")],
            ))
            .unwrap();
        registry
            .apply_inventory(inventory(
                "openai_primary",
                "r2",
                vec![model("openai_primary", "gpt-5.2", "llm.gpt5")],
            ))
            .unwrap();

        assert_eq!(registry.inventory_revision("openai_primary"), Some("r2"));
        assert!(registry
            .exact_candidate("gpt-5.1@openai_primary", &ApiType::LlmChat)
            .is_none());
        assert!(registry
            .exact_candidate("gpt-5.2@openai_primary", &ApiType::LlmChat)
            .is_some());
    }

    #[test]
    fn default_items_generation_is_pure() {
        let inv = inventory(
            "openai_primary",
            "r1",
            vec![model("openai_primary", "gpt-5.2", "llm.gpt5")],
        );
        let first = default_items_from_inventories([&inv].into_iter(), "llm.gpt5");
        let second = default_items_from_inventories([&inv].into_iter(), "llm.gpt5");

        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(
            first
                .get("gpt-5_2_openai_primary")
                .map(|item| item.target.as_str()),
            Some("gpt-5.2@openai_primary")
        );
    }
}
