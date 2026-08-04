use crate::model_types::{
    ApiType, ExactModelName, LogicalAdmissionTrace, LogicalItems, LogicalModelDefinition,
    ModelCandidate, ModelItem, MountMode, ProviderInventory, RouteError, RouteErrorCode,
};
use log::warn;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Notify;

pub const DEFAULT_INVENTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default)]
pub struct ModelRegistry {
    inventories: HashMap<String, ProviderInventory>,
    exact_index: HashMap<(String, ApiType), ModelCandidate>,
    logical_definitions: BTreeMap<String, LogicalModelDefinition>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_inventory(&mut self, inventory: ProviderInventory) -> Result<(), RouteError> {
        self.apply_inventory_if_changed(inventory).map(|_| ())
    }

    pub fn apply_inventory_if_changed(
        &mut self,
        mut inventory: ProviderInventory,
    ) -> Result<bool, RouteError> {
        for model in inventory.models.iter_mut() {
            if model.model_driver.trim().is_empty() {
                model.model_driver = inventory.provider_driver.clone();
            }
        }
        validate_inventory(&inventory)?;
        let provider_instance_name = inventory.provider_instance_name.clone();
        if let Some(current) = self.inventories.get(provider_instance_name.as_str()) {
            if inventory.driver_metadata_generation < current.driver_metadata_generation {
                return Ok(false);
            }
            if inventory.driver_metadata_generation == current.driver_metadata_generation
                && inventory.inventory_revision.is_some()
                && current.inventory_revision == inventory.inventory_revision
            {
                return Ok(false);
            }
        }

        let mut inventories = self.inventories.clone();
        inventories.insert(provider_instance_name, inventory);
        let exact_index = build_exact_index(inventories.values())?;
        self.inventories = inventories;
        self.exact_index = exact_index;
        Ok(true)
    }

    pub fn remove_inventory(&mut self, provider_instance_name: &str) -> Result<(), RouteError> {
        self.inventories.remove(provider_instance_name);
        self.rebuild_index()
    }

    pub fn clear(&mut self) {
        self.inventories.clear();
        self.exact_index.clear();
    }

    pub fn set_logical_definitions(
        &mut self,
        definitions: Vec<LogicalModelDefinition>,
    ) -> Result<(), RouteError> {
        let mut next = BTreeMap::new();
        for definition in definitions {
            validate_logical_definition(&definition)?;
            next.insert(definition.path.clone(), definition);
        }
        self.logical_definitions = next;
        Ok(())
    }

    pub fn logical_definition(&self, path: &str) -> Option<&LogicalModelDefinition> {
        self.logical_definitions.get(path)
    }

    pub fn logical_definitions(&self) -> impl Iterator<Item = &LogicalModelDefinition> {
        self.logical_definitions.values()
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
        self.default_items_with_trace_for_path(logical_path).items
    }

    pub fn default_items_with_trace_for_path(&self, logical_path: &str) -> DefaultLogicalItems {
        default_items_from_inventories(
            self.inventories.values(),
            logical_path,
            self.logical_definitions.get(logical_path),
        )
    }

    pub fn all_default_items(&self) -> BTreeMap<String, LogicalItems> {
        let mut mounts = BTreeMap::<String, LogicalItems>::new();
        for inventory in self.inventories.values() {
            for model in inventory.models.iter() {
                for mount in model.logical_mounts.iter() {
                    let items = self.default_items_for_path(mount);
                    if !items.is_empty() {
                        mounts.insert(mount.clone(), items);
                    }
                }
            }
        }
        for path in self.logical_definitions.keys() {
            let items = self.default_items_for_path(path);
            mounts.insert(path.clone(), items);
        }
        mounts
    }

    fn rebuild_index(&mut self) -> Result<(), RouteError> {
        self.exact_index = build_exact_index(self.inventories.values())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct DefaultLogicalItems {
    pub items: LogicalItems,
    pub item_sources: BTreeMap<String, String>,
    pub admission: Vec<LogicalAdmissionTrace>,
}

fn build_exact_index<'a>(
    inventories: impl Iterator<Item = &'a ProviderInventory>,
) -> Result<HashMap<(String, ApiType), ModelCandidate>, RouteError> {
    let mut next = HashMap::new();
    for inventory in inventories {
        validate_inventory(inventory)?;
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
    Ok(next)
}

pub struct InventoryRefreshScheduler {
    registry: Arc<RwLock<ModelRegistry>>,
    inventory_source: Arc<dyn Fn() -> Vec<ProviderInventory> + Send + Sync>,
    refresh_hook: Option<Arc<dyn Fn() -> Result<(), RouteError> + Send + Sync>>,
    interval: Duration,
    notify: Notify,
    started: AtomicBool,
}

impl InventoryRefreshScheduler {
    pub fn new(
        registry: Arc<RwLock<ModelRegistry>>,
        inventory_source: Arc<dyn Fn() -> Vec<ProviderInventory> + Send + Sync>,
        interval: Duration,
    ) -> Self {
        Self {
            registry,
            inventory_source,
            refresh_hook: None,
            interval,
            notify: Notify::new(),
            started: AtomicBool::new(false),
        }
    }

    pub fn with_refresh_hook(
        mut self,
        hook: Arc<dyn Fn() -> Result<(), RouteError> + Send + Sync>,
    ) -> Self {
        self.refresh_hook = Some(hook);
        self
    }

    pub fn refresh_once(&self) -> Result<usize, RouteError> {
        if let Some(hook) = self.refresh_hook.as_ref() {
            hook()?;
        }
        let inventories = (self.inventory_source)();
        let active_providers = inventories
            .iter()
            .map(|inventory| inventory.provider_instance_name.clone())
            .collect::<HashSet<_>>();
        let mut changed = 0;
        let mut registry = self.registry.write().map_err(|_| {
            RouteError::new(
                RouteErrorCode::ProviderUnavailable,
                "registry lock poisoned",
            )
        })?;
        // 一个 provider 的 inventory 校验失败（比如 SessionConfigInvalid /
        // 重复 exact_model）只能让那个 provider 不更新，不能连累其它 provider。
        // 早期实现用 `?` 直接传播错误，结果一条坏 inventory 会让循环里它后面
        // 的 provider 也都跳过 apply，registry 会停留在很久以前装载好的那份
        // 快照上。这里改成 per-provider try/log。
        for inventory in inventories {
            let provider_instance_name = inventory.provider_instance_name.clone();
            match registry.apply_inventory_if_changed(inventory) {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        "aicc.model_registry.apply_inventory_failed provider_instance_name={} err={}",
                        provider_instance_name, err
                    );
                }
            }
        }
        let stale_providers = registry
            .inventories()
            .filter(|inventory| !active_providers.contains(&inventory.provider_instance_name))
            .map(|inventory| inventory.provider_instance_name.clone())
            .collect::<Vec<_>>();
        for provider in stale_providers {
            if let Err(err) = registry.remove_inventory(provider.as_str()) {
                warn!(
                    "aicc.model_registry.remove_inventory_failed provider_instance_name={} err={}",
                    provider, err
                );
                continue;
            }
            changed += 1;
        }
        Ok(changed)
    }

    pub fn inventory_changed(&self) {
        self.notify.notify_one();
    }

    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(scheduler.interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let _ = scheduler.refresh_once();
                    }
                    _ = scheduler.notify.notified() => {
                        let _ = scheduler.refresh_once();
                    }
                }
            }
        });
    }
}

pub fn default_items_from_inventories<'a>(
    inventories: impl Iterator<Item = &'a ProviderInventory>,
    logical_path: &str,
    definition: Option<&LogicalModelDefinition>,
) -> DefaultLogicalItems {
    let mut items = BTreeMap::<String, ModelItem>::new();
    let mut item_sources = BTreeMap::<String, String>::new();
    let mut admission = Vec::new();
    for inventory in inventories {
        for model in inventory.models.iter() {
            let has_metadata_mount = model
                .logical_mounts
                .iter()
                .any(|mount| mount.as_str() == logical_path);
            let mut mounted_by_metadata = false;
            if has_metadata_mount {
                let source = "driver_metadata_mount";
                if let Some(definition) = definition {
                    let reasons = admission_rejection_reasons(definition, model);
                    admission.push(LogicalAdmissionTrace {
                        logical_path: logical_path.to_string(),
                        exact_model: model.exact_model.clone(),
                        source: source.to_string(),
                        accepted: reasons.is_empty(),
                        reasons: reasons.clone(),
                    });
                    if !reasons.is_empty() {
                        continue;
                    }
                }
                let item_name = default_item_name(model.exact_model.as_str());
                items.insert(item_name, ModelItem::new(model.exact_model.clone(), 1.0));
                item_sources.insert(
                    default_item_name(model.exact_model.as_str()),
                    source.to_string(),
                );
                mounted_by_metadata = true;
            }

            let Some(definition) = definition else {
                continue;
            };
            if mounted_by_metadata || definition.mount_mode == MountMode::Manual {
                continue;
            }
            let source = "auto_admission";
            let reasons = admission_rejection_reasons(definition, model);
            admission.push(LogicalAdmissionTrace {
                logical_path: logical_path.to_string(),
                exact_model: model.exact_model.clone(),
                source: source.to_string(),
                accepted: reasons.is_empty(),
                reasons: reasons.clone(),
            });
            if reasons.is_empty() {
                let item_name = default_item_name(model.exact_model.as_str());
                items.insert(
                    item_name.clone(),
                    ModelItem::new(model.exact_model.clone(), 1.0),
                );
                item_sources.insert(item_name, source.to_string());
            }
        }
    }
    DefaultLogicalItems {
        items,
        item_sources,
        admission,
    }
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

fn validate_logical_definition(definition: &LogicalModelDefinition) -> Result<(), RouteError> {
    if definition.path.trim().is_empty() || definition.path.contains('@') {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "logical definition path must be a logical model path",
        ));
    }
    if definition
        .path
        .split('.')
        .any(|part| part.trim().is_empty())
    {
        return Err(RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            "logical definition path contains an empty segment",
        ));
    }
    Ok(())
}

fn admission_rejection_reasons(
    definition: &LogicalModelDefinition,
    model: &crate::model_types::ModelMetadata,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if !model.supports_api_type(&definition.api_type) {
        reasons.push("api_type_mismatch".to_string());
    }
    reasons.extend(
        model
            .capabilities
            .explain_missing_requirements(&definition.min_line),
    );
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_types::{
        CostClass, HealthStatus, LogicalModelDefinition, ModelAttributes, ModelCapabilities,
        ModelHealth, ModelMetadata, ModelRequirement, MountMode, ProviderType,
    };
    use std::sync::atomic::AtomicUsize;

    fn model(provider: &str, provider_model_id: &str, mount: &str) -> ModelMetadata {
        ModelMetadata {
            provider_model_id: provider_model_id.to_string(),
            exact_model: format!("{}@{}", provider_model_id, provider),
            model_driver: provider.to_string(),
            origin_model_id: None,
            provider_actual_model_id: None,
            provider_options: None,
            parameter_scale: None,
            api_types: vec![ApiType::Llm],
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
            provider_driver: "test".to_string(),
            provider_origin: Default::default(),
            provider_type_trusted_source: Default::default(),
            provider_type_revision: None,
            version: None,
            inventory_revision: Some(revision.to_string()),
            driver_metadata_generation: 0,
            models,
        }
    }

    fn logical_definition(path: &str, min_line: ModelRequirement) -> LogicalModelDefinition {
        LogicalModelDefinition {
            path: path.to_string(),
            api_type: ApiType::Llm,
            min_line,
            disable_line: Default::default(),
            default_options: None,
            mount_mode: MountMode::Auto,
            scheduler_profile: None,
            fallback: None,
            route_policy: None,
            user_visible_tier: None,
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
            .exact_candidate("gpt-5.1@openai_primary", &ApiType::Llm)
            .is_none());
        assert!(registry
            .exact_candidate("gpt-5.2@openai_primary", &ApiType::Llm)
            .is_some());
    }

    #[test]
    fn metadata_generation_replaces_same_inventory_revision() {
        let mut registry = ModelRegistry::new();
        let mut first = inventory(
            "openai_primary",
            "models-r1",
            vec![model("openai_primary", "gpt-5.2", "llm.old")],
        );
        first.driver_metadata_generation = 1;
        assert!(registry.apply_inventory_if_changed(first).unwrap());

        let mut updated = inventory(
            "openai_primary",
            "models-r1",
            vec![model("openai_primary", "gpt-5.2", "llm.new")],
        );
        updated.driver_metadata_generation = 2;
        assert!(registry.apply_inventory_if_changed(updated).unwrap());
        assert!(registry.default_items_for_path("llm.old").is_empty());
        assert_eq!(registry.default_items_for_path("llm.new").len(), 1);

        let mut stale = inventory(
            "openai_primary",
            "models-r2",
            vec![model("openai_primary", "gpt-5.2", "llm.stale")],
        );
        stale.driver_metadata_generation = 1;
        assert!(!registry.apply_inventory_if_changed(stale).unwrap());
        assert!(registry.default_items_for_path("llm.stale").is_empty());
        assert_eq!(registry.default_items_for_path("llm.new").len(), 1);
    }

    #[test]
    fn refresh_once_skips_bad_provider_and_keeps_others() {
        // 之前的实现会让一个 provider 校验失败连累循环里它后面的 provider，
        // 这里证明现在每个 provider 独立处理。
        let registry = Arc::new(RwLock::new(ModelRegistry::new()));
        let bad = inventory(
            "openai_primary",
            "r1",
            vec![
                model("openai_primary", "gpt-5.2", "llm.gpt5"),
                model("openai_primary", "gpt-5.2", "llm.plan"), // duplicate
            ],
        );
        let good = inventory(
            "google_primary",
            "r1",
            vec![model("google_primary", "gemini-2.5-flash", "llm.vision")],
        );
        let inventories = Arc::new(vec![bad, good]);
        let scheduler = InventoryRefreshScheduler::new(
            registry.clone(),
            Arc::new(move || (*inventories).clone()),
            Duration::from_secs(60),
        );

        let changed = scheduler.refresh_once().unwrap();
        assert_eq!(changed, 1, "only the good provider should apply");

        let guard = registry.read().unwrap();
        assert!(
            guard
                .exact_candidate("gemini-2.5-flash@google_primary", &ApiType::Llm)
                .is_some(),
            "google inventory should be in the registry despite openai failing"
        );
        assert!(
            guard
                .exact_candidate("gpt-5.2@openai_primary", &ApiType::Llm)
                .is_none(),
            "openai inventory should not be applied (it was malformed)"
        );
    }

    #[test]
    fn refresh_once_runs_refresh_hook() {
        let registry = Arc::new(RwLock::new(ModelRegistry::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_hook = calls.clone();
        let inventories = Arc::new(vec![inventory(
            "openai_primary",
            "r1",
            vec![model("openai_primary", "gpt-5.2", "llm.gpt5")],
        )]);
        let scheduler = InventoryRefreshScheduler::new(
            registry.clone(),
            Arc::new(move || (*inventories).clone()),
            Duration::from_secs(60),
        )
        .with_refresh_hook(Arc::new(move || {
            calls_for_hook.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));

        assert_eq!(scheduler.refresh_once().unwrap(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn default_items_generation_is_pure() {
        let inv = inventory(
            "openai_primary",
            "r1",
            vec![model("openai_primary", "gpt-5.2", "llm.gpt5")],
        );
        let first = default_items_from_inventories([&inv].into_iter(), "llm.gpt5", None);
        let second = default_items_from_inventories([&inv].into_iter(), "llm.gpt5", None);

        assert_eq!(
            serde_json::to_value(&first.items).unwrap(),
            serde_json::to_value(&second.items).unwrap()
        );
        assert_eq!(
            first
                .items
                .get("gpt-5_2_openai_primary")
                .map(|item| item.target.as_str()),
            Some("gpt-5.2@openai_primary")
        );
    }

    #[test]
    fn auto_mount_uses_min_line_admission() {
        let mut accepted = model("openai_primary", "gpt-5.2", "");
        accepted.logical_mounts = Vec::new();
        accepted.capabilities = ModelCapabilities {
            tool_call: true,
            json_schema: true,
            max_context_tokens: Some(128_000),
            ..Default::default()
        };
        let mut missing_tool = model("openai_primary", "small", "");
        missing_tool.logical_mounts = Vec::new();
        missing_tool.capabilities = ModelCapabilities {
            json_schema: true,
            max_context_tokens: Some(128_000),
            ..Default::default()
        };
        let mut registry = ModelRegistry::new();
        registry
            .set_logical_definitions(vec![logical_definition(
                "llm.plan",
                ModelRequirement {
                    tool_call: true,
                    json_schema: true,
                    min_context_tokens: Some(32_768),
                    ..Default::default()
                },
            )])
            .unwrap();
        registry
            .apply_inventory(inventory(
                "openai_primary",
                "r1",
                vec![accepted, missing_tool],
            ))
            .unwrap();

        let defaults = registry.default_items_with_trace_for_path("llm.plan");
        assert_eq!(defaults.items.len(), 1);
        assert!(defaults
            .items
            .values()
            .any(|item| item.target == "gpt-5.2@openai_primary"));
        assert!(defaults
            .admission
            .iter()
            .any(|item| item.exact_model == "small@openai_primary"
                && !item.accepted
                && item.reasons.iter().any(|reason| reason == "tool_call")));
    }

    #[test]
    fn manual_mount_mode_disables_auto_mount() {
        let mut auto_candidate = model("openai_primary", "gpt-5.2", "");
        auto_candidate.logical_mounts = Vec::new();
        auto_candidate.capabilities = ModelCapabilities {
            tool_call: true,
            json_schema: true,
            max_context_tokens: Some(128_000),
            ..Default::default()
        };
        let mut definition = logical_definition("llm.plan", ModelRequirement::default());
        definition.mount_mode = MountMode::Manual;
        let mut registry = ModelRegistry::new();
        registry.set_logical_definitions(vec![definition]).unwrap();
        registry
            .apply_inventory(inventory("openai_primary", "r1", vec![auto_candidate]))
            .unwrap();

        assert!(registry.default_items_for_path("llm.plan").is_empty());
    }

    #[test]
    fn all_default_items_keeps_empty_logical_definitions() {
        let mut registry = ModelRegistry::new();
        registry
            .set_logical_definitions(vec![logical_definition(
                "llm.plan",
                ModelRequirement::default(),
            )])
            .unwrap();

        let defaults = registry.all_default_items();
        assert!(
            defaults.contains_key("llm.plan"),
            "configured logical definitions should remain visible even without admitted models"
        );
        assert!(defaults.get("llm.plan").unwrap().is_empty());
    }
}
