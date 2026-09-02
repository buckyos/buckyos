#![allow(dead_code)]

use crate::catalog::CatalogSnapshot;
use buckyos_api::{
    AiccFallbackMode, AiccFallbackRule, AiccLogicalNodeOverlay, AiccLogicalTreeOverlay,
    AiccPolicyConfig, AiccRouteOverlay, AiccSchedulerProfile, ApiType, ModelDisable, ModelItem,
    ModelItemPatch, ModelRequirement, OverlayMergeMode,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub(crate) const DEFAULT_FALLBACK_DEPTH_LIMIT: usize = 5;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ExactModelName {
    provider_model_id: String,
    variant: Option<String>,
    provider_instance_name: String,
    rendered: String,
}

impl ExactModelName {
    pub(crate) fn new(
        provider_model_id: impl Into<String>,
        variant: Option<String>,
        provider_instance_name: impl Into<String>,
    ) -> Result<Self, ModelRegistryError> {
        let provider_model_id = provider_model_id.into();
        let provider_instance_name = provider_instance_name.into();
        validate_identity("provider_model_id", &provider_model_id, false)?;
        validate_identity("provider_instance_name", &provider_instance_name, false)?;
        if let Some(variant) = &variant {
            validate_variant(variant)?;
        }
        let model_part = variant.as_ref().map_or_else(
            || provider_model_id.clone(),
            |variant| format!("{provider_model_id}:{variant}"),
        );
        Ok(Self {
            rendered: format!("{model_part}@{provider_instance_name}"),
            provider_model_id,
            variant,
            provider_instance_name,
        })
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ModelRegistryError> {
        let mut parts = value.split('@');
        let model_part = parts.next().unwrap_or_default();
        let instance = parts.next().unwrap_or_default();
        if model_part.is_empty() || instance.is_empty() || parts.next().is_some() {
            return Err(ModelRegistryError::InvalidExactModelName(value.to_owned()));
        }
        let (provider_model_id, variant) = model_part
            .rsplit_once(':')
            .map(|(model, variant)| (model, Some(variant.to_owned())))
            .unwrap_or((model_part, None));
        Self::new(provider_model_id, variant, instance)
    }

    pub(crate) fn provider_model_id(&self) -> &str {
        &self.provider_model_id
    }

    pub(crate) fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    pub(crate) fn provider_instance_name(&self) -> &str {
        &self.provider_instance_name
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.rendered
    }
}

impl fmt::Display for ExactModelName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.rendered)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ModelUid {
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub protocol_adapter_id: String,
    pub variant: Option<String>,
}

impl ModelUid {
    pub(crate) fn new(
        model_driver_id: impl Into<String>,
        origin_model_id: impl Into<String>,
        protocol_adapter_id: impl Into<String>,
        variant: Option<String>,
    ) -> Result<Self, ModelRegistryError> {
        let value = Self {
            model_driver_id: model_driver_id.into(),
            origin_model_id: origin_model_id.into(),
            protocol_adapter_id: protocol_adapter_id.into(),
            variant,
        };
        validate_non_empty("model_driver_id", &value.model_driver_id)?;
        validate_non_empty("origin_model_id", &value.origin_model_id)?;
        validate_non_empty("protocol_adapter_id", &value.protocol_adapter_id)?;
        if let Some(variant) = &value.variant {
            validate_variant(variant)?;
        }
        Ok(value)
    }

    pub(crate) fn as_stable_string(&self) -> String {
        let mut value = format!(
            "{}:{}:{}",
            encode_uid_component(&self.model_driver_id),
            encode_uid_component(&self.origin_model_id),
            encode_uid_component(&self.protocol_adapter_id)
        );
        if let Some(variant) = &self.variant {
            value.push(':');
            value.push_str(&encode_uid_component(variant));
        }
        value
    }
}

impl fmt::Display for ModelUid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_stable_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderModelIdentity {
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub provider_model_id: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InventoryModelVariant {
    pub name: String,
    pub logical_mounts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InventoryModel {
    pub provider_model_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub api_types: Vec<ApiType>,
    pub logical_mounts: Vec<String>,
    pub variants: Vec<InventoryModelVariant>,
    pub capabilities: BTreeMap<String, Value>,
    pub attributes: BTreeMap<String, Value>,
    pub operations: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderInventory {
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub inventory_revision: String,
    pub models: Vec<InventoryModel>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MountMode {
    Manual,
    Auto,
    #[default]
    Hybrid,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogicalModelDefinition {
    pub path: String,
    pub api_type: ApiType,
    pub min_line: ModelRequirement,
    pub disable_line: ModelDisable,
    pub default_options: BTreeMap<String, Value>,
    pub mount_mode: MountMode,
    pub scheduler_profile: AiccSchedulerProfile,
    pub fallback: Option<AiccFallbackRule>,
    pub route_policy: AiccPolicyConfig,
    pub user_visible_tier: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum LogicalItemSource {
    BuiltinDefinition,
    DriverMetadataMount,
    AutoAdmission,
    ManualOverride,
    UserOverlay,
    SessionOverlay,
}

#[derive(Clone, Debug, PartialEq)]
struct EffectiveItem {
    item: ModelItem,
    source: LogicalItemSource,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EffectiveLogicalNode {
    definition: Option<LogicalModelDefinition>,
    items: BTreeMap<String, EffectiveItem>,
    exact_model_weights: BTreeMap<String, f64>,
    disable_line: ModelDisable,
    fallback: Option<AiccFallbackRule>,
    admissions: BTreeMap<String, AdmissionRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegisteredModel {
    pub exact_model: ExactModelName,
    pub model_uid: ModelUid,
    pub identity: ProviderModelIdentity,
    pub api_types: Vec<ApiType>,
    pub logical_mounts: Vec<String>,
    pub capabilities: BTreeMap<String, Value>,
    pub attributes: BTreeMap<String, Value>,
    pub operations: BTreeMap<String, String>,
    pub inventory_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelView {
    pub exact_model: String,
    pub model_uid: String,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub provider_model_id: String,
    pub variant: Option<String>,
    pub api_types: Vec<String>,
    pub logical_mounts: Vec<String>,
    pub capabilities: BTreeMap<String, Value>,
    pub attributes: BTreeMap<String, Value>,
    pub operations: BTreeMap<String, String>,
    pub inventory_revision: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogicalItemView {
    pub name: String,
    pub target: String,
    pub weight: f64,
    pub source: LogicalItemSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogicalModelView {
    pub path: String,
    pub api_type: Option<String>,
    pub mount_mode: Option<MountMode>,
    pub item_count: usize,
    pub items: Vec<LogicalItemView>,
    pub min_line: ModelRequirement,
    pub disable_line: ModelDisable,
    pub default_options: BTreeMap<String, Value>,
    pub scheduler_profile: Option<AiccSchedulerProfile>,
    pub fallback: Option<AiccFallbackRule>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CandidatePath {
    pub logical_paths: Vec<String>,
    pub item_names: Vec<String>,
    pub priority: Vec<f64>,
    pub sources: Vec<LogicalItemSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegistryCandidate {
    pub model: RegisteredModel,
    pub paths: Vec<CandidatePath>,
    pub exact_model_weight: f64,
    pub provider_weight: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmissionRecord {
    pub logical_path: String,
    pub exact_model: String,
    pub admitted: bool,
    pub missing_requirements: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FallbackStep {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CandidateSet {
    pub requested_logical_path: String,
    pub resolved_logical_path: String,
    pub candidates: Vec<RegistryCandidate>,
    pub admissions: Vec<AdmissionRecord>,
    pub fallback_chain: Vec<FallbackStep>,
    pub disable_line: ModelDisable,
    pub default_options: BTreeMap<String, Value>,
    pub scheduler_profile: AiccSchedulerProfile,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RegistryLayers<'a> {
    pub factory: Option<&'a AiccRouteOverlay>,
    pub system: Option<&'a AiccRouteOverlay>,
    pub user: Option<&'a AiccRouteOverlay>,
    pub session: Option<&'a AiccRouteOverlay>,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelRegistry {
    models: BTreeMap<String, RegisteredModel>,
    logical_nodes: BTreeMap<String, EffectiveLogicalNode>,
    global_exact_model_weights: BTreeMap<String, f64>,
    provider_weights: BTreeMap<String, f64>,
    fallback_depth_limit: usize,
}

impl ModelRegistry {
    pub(crate) fn build(
        catalog: &CatalogSnapshot,
        inventories: &[ProviderInventory],
        definitions: Vec<LogicalModelDefinition>,
        layers: RegistryLayers<'_>,
    ) -> Result<Self, ModelRegistryError> {
        let mut registry = Self {
            models: BTreeMap::new(),
            logical_nodes: BTreeMap::new(),
            global_exact_model_weights: BTreeMap::new(),
            provider_weights: BTreeMap::new(),
            fallback_depth_limit: DEFAULT_FALLBACK_DEPTH_LIMIT,
        };
        registry.register_definitions(definitions)?;
        registry.register_inventories(catalog, inventories)?;
        registry.materialize_driver_mounts();
        registry.materialize_auto_mounts();
        for (layer, source) in [
            (layers.factory, LogicalItemSource::BuiltinDefinition),
            (layers.system, LogicalItemSource::ManualOverride),
            (layers.user, LogicalItemSource::UserOverlay),
            (layers.session, LogicalItemSource::SessionOverlay),
        ] {
            if let Some(layer) = layer {
                registry.apply_route_overlay(layer, source)?;
            }
        }
        registry.validate_item_graph()?;
        registry.validate_fallback_graph()?;
        Ok(registry)
    }

    pub(crate) fn exact_model(&self, exact_model: &str) -> Option<&RegisteredModel> {
        self.models.get(exact_model)
    }

    pub(crate) fn model_views(&self) -> Vec<ModelView> {
        self.models.values().map(ModelView::from).collect()
    }

    pub(crate) fn logical_model_views(&self) -> Vec<LogicalModelView> {
        self.logical_nodes
            .iter()
            .map(|(path, node)| LogicalModelView {
                path: path.clone(),
                api_type: node
                    .definition
                    .as_ref()
                    .map(|definition| api_type_name(definition.api_type).to_owned()),
                mount_mode: node
                    .definition
                    .as_ref()
                    .map(|definition| definition.mount_mode),
                item_count: node.items.len(),
                items: node
                    .items
                    .iter()
                    .map(|(name, item)| LogicalItemView {
                        name: name.clone(),
                        target: item.item.target.clone(),
                        weight: item.item.weight,
                        source: item.source,
                    })
                    .collect(),
                min_line: node
                    .definition
                    .as_ref()
                    .map(|definition| definition.min_line.clone())
                    .unwrap_or_default(),
                disable_line: node.disable_line.clone(),
                default_options: node
                    .definition
                    .as_ref()
                    .map(|definition| definition.default_options.clone())
                    .unwrap_or_default(),
                scheduler_profile: node
                    .definition
                    .as_ref()
                    .map(|definition| definition.scheduler_profile.clone()),
                fallback: node.fallback.clone(),
            })
            .collect()
    }

    pub(crate) fn resolve_candidates(
        &self,
        logical_path: &str,
        api_type: ApiType,
    ) -> Result<CandidateSet, ModelRegistryError> {
        validate_logical_path(logical_path)?;
        ensure_path_api_namespace(logical_path, api_type)?;
        let requested = logical_path.to_owned();
        let mut current = requested.clone();
        let mut fallback_chain = Vec::new();
        let mut all_admissions = Vec::new();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return Err(ModelRegistryError::FallbackLoop(current));
            }
            if fallback_chain.len() > self.fallback_depth_limit {
                return Err(ModelRegistryError::FallbackDepthExceeded(
                    self.fallback_depth_limit,
                ));
            }
            let (candidates, admissions) = self.expand(&current, api_type)?;
            all_admissions.extend(admissions);
            normalize_admissions(&mut all_admissions);
            if !candidates.is_empty() {
                let disable_line = self.disable_line(&current);
                let default_options = self.default_options(&current);
                let scheduler_profile = self.scheduler_profile(&current);
                return Ok(CandidateSet {
                    requested_logical_path: requested,
                    resolved_logical_path: current,
                    candidates,
                    admissions: all_admissions,
                    fallback_chain,
                    disable_line,
                    default_options,
                    scheduler_profile,
                });
            }
            match self.fallback_target(&current, api_type)? {
                FallbackTarget::None => {
                    let disable_line = self.disable_line(&current);
                    let default_options = self.default_options(&current);
                    let scheduler_profile = self.scheduler_profile(&current);
                    return Ok(CandidateSet {
                        requested_logical_path: requested,
                        resolved_logical_path: current,
                        candidates,
                        admissions: all_admissions,
                        fallback_chain,
                        disable_line,
                        default_options,
                        scheduler_profile,
                    });
                }
                FallbackTarget::Logical(next) => {
                    ensure_path_api_namespace(&next, api_type)?;
                    fallback_chain.push(FallbackStep {
                        from: current,
                        to: next.clone(),
                    });
                    current = next;
                }
                FallbackTarget::Exact(exact) => {
                    let candidates = self.exact_fallback_candidate(&exact, api_type);
                    let disable_line = self.disable_line(&current);
                    fallback_chain.push(FallbackStep {
                        from: current,
                        to: exact.clone(),
                    });
                    return Ok(CandidateSet {
                        requested_logical_path: requested,
                        resolved_logical_path: exact,
                        candidates,
                        admissions: all_admissions,
                        fallback_chain,
                        disable_line,
                        default_options: BTreeMap::new(),
                        scheduler_profile: AiccSchedulerProfile::Balanced,
                    });
                }
            }
        }
    }

    fn register_definitions(
        &mut self,
        definitions: Vec<LogicalModelDefinition>,
    ) -> Result<(), ModelRegistryError> {
        for definition in definitions {
            validate_logical_path(&definition.path)?;
            ensure_path_api_namespace(&definition.path, definition.api_type)?;
            validate_fallback_rule(definition.fallback.as_ref())?;
            let path = definition.path.clone();
            let node = self.logical_nodes.entry(path.clone()).or_default();
            if node.definition.is_some() {
                return Err(ModelRegistryError::DuplicateLogicalDefinition(path));
            }
            node.disable_line = definition.disable_line.clone();
            node.fallback = definition.fallback.clone();
            node.definition = Some(definition);
        }
        Ok(())
    }

    fn register_inventories(
        &mut self,
        catalog: &CatalogSnapshot,
        inventories: &[ProviderInventory],
    ) -> Result<(), ModelRegistryError> {
        let mut instances = BTreeSet::new();
        for inventory in inventories {
            validate_identity(
                "provider_instance_name",
                &inventory.provider_instance_name,
                false,
            )?;
            validate_non_empty("provider_profile_id", &inventory.provider_profile_id)?;
            validate_non_empty("protocol_adapter_id", &inventory.protocol_adapter_id)?;
            validate_non_empty("inventory_revision", &inventory.inventory_revision)?;
            if !instances.insert(inventory.provider_instance_name.clone()) {
                return Err(ModelRegistryError::DuplicateProviderInstance(
                    inventory.provider_instance_name.clone(),
                ));
            }
            for model in &inventory.models {
                if catalog.model_driver(&model.model_driver_id).is_none() {
                    return Err(ModelRegistryError::UnknownModelDriver(
                        model.model_driver_id.clone(),
                    ));
                }
                self.register_model(inventory, model, None)?;
                let mut variants = BTreeSet::new();
                for variant in &model.variants {
                    validate_variant(&variant.name)?;
                    if !variants.insert(variant.name.clone()) {
                        return Err(ModelRegistryError::DuplicateVariant {
                            provider_model_id: model.provider_model_id.clone(),
                            variant: variant.name.clone(),
                        });
                    }
                    self.register_model(inventory, model, Some(variant))?;
                }
            }
        }
        Ok(())
    }

    fn register_model(
        &mut self,
        inventory: &ProviderInventory,
        model: &InventoryModel,
        variant: Option<&InventoryModelVariant>,
    ) -> Result<(), ModelRegistryError> {
        if model.api_types.is_empty() {
            return Err(ModelRegistryError::MissingApiTypes(
                model.provider_model_id.clone(),
            ));
        }
        let variant_name = variant.map(|variant| variant.name.clone());
        let exact_model = ExactModelName::new(
            &model.provider_model_id,
            variant_name.clone(),
            &inventory.provider_instance_name,
        )?;
        let model_uid = ModelUid::new(
            &model.model_driver_id,
            &model.origin_model_id,
            &inventory.protocol_adapter_id,
            variant_name,
        )?;
        let mut logical_mounts = model.logical_mounts.clone();
        if let Some(variant) = variant {
            logical_mounts.extend(variant.logical_mounts.iter().cloned());
        }
        logical_mounts.sort();
        logical_mounts.dedup();
        for path in &logical_mounts {
            validate_logical_path(path)?;
            if !model
                .api_types
                .iter()
                .any(|api_type| path_matches_api_namespace(path, *api_type))
            {
                return Err(ModelRegistryError::MountApiMismatch {
                    path: path.clone(),
                    provider_model_id: model.provider_model_id.clone(),
                });
            }
        }
        let registered = RegisteredModel {
            exact_model: exact_model.clone(),
            model_uid,
            identity: ProviderModelIdentity {
                provider_instance_name: inventory.provider_instance_name.clone(),
                provider_profile_id: inventory.provider_profile_id.clone(),
                protocol_adapter_id: inventory.protocol_adapter_id.clone(),
                model_driver_id: model.model_driver_id.clone(),
                origin_model_id: model.origin_model_id.clone(),
                provider_model_id: model.provider_model_id.clone(),
            },
            api_types: deduplicate_api_types(&model.api_types),
            logical_mounts,
            capabilities: model.capabilities.clone(),
            attributes: model.attributes.clone(),
            operations: model.operations.clone(),
            inventory_revision: inventory.inventory_revision.clone(),
        };
        if self
            .models
            .insert(exact_model.to_string(), registered)
            .is_some()
        {
            return Err(ModelRegistryError::DuplicateExactModel(
                exact_model.to_string(),
            ));
        }
        Ok(())
    }

    fn materialize_driver_mounts(&mut self) {
        let models = self.models.values().cloned().collect::<Vec<_>>();
        for model in models {
            for path in &model.logical_mounts {
                if model
                    .api_types
                    .iter()
                    .any(|api_type| path_matches_api_namespace(path, *api_type))
                {
                    let missing = self.missing_for_path(path, &model);
                    self.record_admission(path, &model, &missing);
                    if missing.is_empty() {
                        self.insert_default_item(
                            path,
                            &model.exact_model.to_string(),
                            LogicalItemSource::DriverMetadataMount,
                        );
                    }
                }
            }
        }
    }

    fn materialize_auto_mounts(&mut self) {
        let definitions = self
            .logical_nodes
            .values()
            .filter_map(|node| node.definition.clone())
            .filter(|definition| {
                matches!(definition.mount_mode, MountMode::Auto | MountMode::Hybrid)
            })
            .collect::<Vec<_>>();
        let models = self.models.values().cloned().collect::<Vec<_>>();
        for definition in definitions {
            for model in &models {
                if !model.api_types.contains(&definition.api_type) {
                    continue;
                }
                let missing = missing_requirements(&definition.min_line, model);
                self.record_admission(&definition.path, model, &missing);
                if missing.is_empty() {
                    self.insert_default_item(
                        &definition.path,
                        &model.exact_model.to_string(),
                        LogicalItemSource::AutoAdmission,
                    );
                }
            }
        }
    }

    fn insert_default_item(&mut self, path: &str, exact_model: &str, source: LogicalItemSource) {
        self.logical_nodes
            .entry(path.to_owned())
            .or_default()
            .items
            .entry(exact_model.to_owned())
            .or_insert_with(|| EffectiveItem {
                item: ModelItem::new(exact_model, 1.0),
                source,
            });
    }

    fn missing_for_path(&self, path: &str, model: &RegisteredModel) -> Vec<String> {
        self.logical_nodes
            .get(path)
            .and_then(|node| node.definition.as_ref())
            .map(|definition| missing_requirements(&definition.min_line, model))
            .unwrap_or_default()
    }

    fn record_admission(&mut self, path: &str, model: &RegisteredModel, missing: &[String]) {
        self.logical_nodes
            .entry(path.to_owned())
            .or_default()
            .admissions
            .insert(
                model.exact_model.to_string(),
                AdmissionRecord {
                    logical_path: path.to_owned(),
                    exact_model: model.exact_model.to_string(),
                    admitted: missing.is_empty(),
                    missing_requirements: missing.to_vec(),
                },
            );
    }

    fn apply_route_overlay(
        &mut self,
        overlay: &AiccRouteOverlay,
        source: LogicalItemSource,
    ) -> Result<(), ModelRegistryError> {
        validate_weight_map(
            &overlay.global_exact_model_weights,
            "global_exact_model_weights",
        )?;
        validate_weight_map(&overlay.provider_weights, "provider_weights")?;
        self.global_exact_model_weights
            .extend(overlay.global_exact_model_weights.clone());
        self.provider_weights
            .extend(overlay.provider_weights.clone());
        for (path, node) in flatten_logical_tree(&overlay.logical_tree)? {
            self.apply_node_overlay(&path, node, source)?;
        }
        if let Some(active) = &overlay.active_logical_profile {
            let profile = overlay
                .logical_profiles
                .get(active)
                .ok_or_else(|| ModelRegistryError::UnknownLogicalProfile(active.clone()))?;
            for tree_overlay in &profile.overlays {
                self.apply_tree_overlay(tree_overlay, source)?;
            }
        }
        if let Some(profile) = &overlay.logical_profile {
            for tree_overlay in &profile.overlays {
                self.apply_tree_overlay(tree_overlay, source)?;
            }
        }
        Ok(())
    }

    fn apply_node_overlay(
        &mut self,
        path: &str,
        overlay: &AiccLogicalNodeOverlay,
        source: LogicalItemSource,
    ) -> Result<(), ModelRegistryError> {
        validate_logical_path(path)?;
        if overlay.items.is_some() && overlay.item_overrides.is_some() {
            return Err(ModelRegistryError::ItemsAndOverridesConflict(
                path.to_owned(),
            ));
        }
        if let Some(items) = &overlay.items {
            validate_items(path, items)?;
        }
        validate_weight_map(&overlay.exact_model_weights, "exact_model_weights")?;
        let node = self.logical_nodes.entry(path.to_owned()).or_default();
        if let Some(items) = &overlay.items {
            node.items = effective_items(items, source);
        }
        if let Some(patches) = &overlay.item_overrides {
            apply_item_patches(path, &mut node.items, patches, source)?;
        }
        node.exact_model_weights
            .extend(overlay.exact_model_weights.clone());
        if let Some(disable) = &overlay.disable_line {
            node.disable_line = disable.clone();
        }
        if let Some(fallback) = &overlay.fallback {
            validate_fallback_rule(Some(fallback))?;
            node.fallback = Some(fallback.clone());
        }
        Ok(())
    }

    fn apply_tree_overlay(
        &mut self,
        overlay: &AiccLogicalTreeOverlay,
        source: LogicalItemSource,
    ) -> Result<(), ModelRegistryError> {
        validate_logical_path(&overlay.path)?;
        if !overlay.items.is_empty() && !overlay.item_overrides.is_empty() {
            return Err(ModelRegistryError::ItemsAndOverridesConflict(
                overlay.path.clone(),
            ));
        }
        validate_items(&overlay.path, &overlay.items)?;
        validate_weight_map(&overlay.exact_model_weights, "exact_model_weights")?;
        let node = self.logical_nodes.entry(overlay.path.clone()).or_default();
        match overlay.merge_mode {
            OverlayMergeMode::Replace => {
                node.items = effective_items(&overlay.items, source);
                node.fallback = overlay.fallback.clone().or_else(disabled_fallback);
            }
            OverlayMergeMode::Inherit => {
                node.items.extend(effective_items(&overlay.items, source));
                if let Some(fallback) = &overlay.fallback {
                    node.fallback = Some(fallback.clone());
                }
            }
        }
        apply_item_patches(
            &overlay.path,
            &mut node.items,
            &overlay.item_overrides,
            source,
        )?;
        node.exact_model_weights
            .extend(overlay.exact_model_weights.clone());
        if let Some(disable) = &overlay.disable_line {
            node.disable_line = disable.clone();
        }
        validate_fallback_rule(node.fallback.as_ref())
    }

    fn expand(
        &self,
        logical_path: &str,
        api_type: ApiType,
    ) -> Result<(Vec<RegistryCandidate>, Vec<AdmissionRecord>), ModelRegistryError> {
        let mut candidates = BTreeMap::new();
        let mut admissions = Vec::new();
        self.expand_path(
            logical_path,
            api_type,
            &mut BTreeSet::new(),
            &mut Vec::new(),
            CandidatePath {
                logical_paths: Vec::new(),
                item_names: Vec::new(),
                priority: Vec::new(),
                sources: Vec::new(),
            },
            &mut candidates,
            &mut admissions,
        )?;
        admissions.sort_by(|left, right| {
            (&left.logical_path, &left.exact_model).cmp(&(&right.logical_path, &right.exact_model))
        });
        admissions.dedup_by(|left, right| {
            left.logical_path == right.logical_path && left.exact_model == right.exact_model
        });
        Ok((candidates.into_values().collect(), admissions))
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_path(
        &self,
        logical_path: &str,
        api_type: ApiType,
        stack: &mut BTreeSet<String>,
        requirements: &mut Vec<ModelRequirement>,
        path: CandidatePath,
        candidates: &mut BTreeMap<String, RegistryCandidate>,
        admissions: &mut Vec<AdmissionRecord>,
    ) -> Result<(), ModelRegistryError> {
        if !stack.insert(logical_path.to_owned()) {
            return Err(ModelRegistryError::LogicalTreeLoop(logical_path.to_owned()));
        }
        let Some(node) = self.logical_nodes.get(logical_path) else {
            stack.remove(logical_path);
            return Ok(());
        };
        admissions.extend(node.admissions.values().cloned());
        if let Some(definition) = &node.definition {
            if definition.api_type != api_type {
                stack.remove(logical_path);
                return Ok(());
            }
            requirements.push(definition.min_line.clone());
        }
        for (item_name, effective) in &node.items {
            if effective.item.weight == 0.0 {
                continue;
            }
            let mut next_path = path.clone();
            next_path.logical_paths.push(logical_path.to_owned());
            next_path.item_names.push(item_name.clone());
            next_path.priority.push(effective.item.weight);
            next_path.sources.push(effective.source);
            if effective.item.target.contains('@') {
                self.add_leaf_candidate(
                    logical_path,
                    &effective.item.target,
                    api_type,
                    requirements,
                    next_path,
                    node,
                    candidates,
                    admissions,
                );
            } else {
                ensure_same_namespace(logical_path, &effective.item.target)?;
                self.expand_path(
                    &effective.item.target,
                    api_type,
                    stack,
                    requirements,
                    next_path,
                    candidates,
                    admissions,
                )?;
            }
        }
        if node.definition.is_some() {
            requirements.pop();
        }
        stack.remove(logical_path);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_leaf_candidate(
        &self,
        logical_path: &str,
        exact_model: &str,
        api_type: ApiType,
        requirements: &[ModelRequirement],
        path: CandidatePath,
        node: &EffectiveLogicalNode,
        candidates: &mut BTreeMap<String, RegistryCandidate>,
        admissions: &mut Vec<AdmissionRecord>,
    ) {
        let Some(model) = self.models.get(exact_model) else {
            return;
        };
        if !model.api_types.contains(&api_type) {
            return;
        }
        let missing = requirements
            .iter()
            .flat_map(|requirement| missing_requirements(requirement, model))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        admissions.push(AdmissionRecord {
            logical_path: logical_path.to_owned(),
            exact_model: exact_model.to_owned(),
            admitted: missing.is_empty(),
            missing_requirements: missing.clone(),
        });
        if !missing.is_empty() {
            return;
        }
        let exact_model_weight = node
            .exact_model_weights
            .get(exact_model)
            .or_else(|| self.global_exact_model_weights.get(exact_model))
            .copied()
            .unwrap_or(1.0);
        if exact_model_weight == 0.0 {
            return;
        }
        candidates
            .entry(exact_model.to_owned())
            .and_modify(|candidate| candidate.paths.push(path.clone()))
            .or_insert_with(|| RegistryCandidate {
                model: model.clone(),
                paths: vec![path],
                exact_model_weight,
                provider_weight: self
                    .provider_weights
                    .get(&model.identity.provider_instance_name)
                    .copied()
                    .unwrap_or(1.0),
            });
    }

    fn exact_fallback_candidate(&self, exact: &str, api_type: ApiType) -> Vec<RegistryCandidate> {
        self.models
            .get(exact)
            .filter(|model| model.api_types.contains(&api_type))
            .map(|model| RegistryCandidate {
                model: model.clone(),
                paths: Vec::new(),
                exact_model_weight: self
                    .global_exact_model_weights
                    .get(exact)
                    .copied()
                    .unwrap_or(1.0),
                provider_weight: self
                    .provider_weights
                    .get(&model.identity.provider_instance_name)
                    .copied()
                    .unwrap_or(1.0),
            })
            .filter(|candidate| candidate.exact_model_weight > 0.0)
            .into_iter()
            .collect()
    }

    fn fallback_target(
        &self,
        path: &str,
        _api_type: ApiType,
    ) -> Result<FallbackTarget, ModelRegistryError> {
        let fallback = self
            .logical_nodes
            .get(path)
            .and_then(|node| node.fallback.as_ref());
        let mode = fallback
            .map(|fallback| &fallback.mode)
            .unwrap_or(&AiccFallbackMode::Parent);
        Ok(match mode {
            AiccFallbackMode::Strict | AiccFallbackMode::Disabled => FallbackTarget::None,
            AiccFallbackMode::Parent => parent_logical_path(path)
                .map(FallbackTarget::Logical)
                .unwrap_or(FallbackTarget::None),
            AiccFallbackMode::TargetLogical => {
                FallbackTarget::Logical(fallback.and_then(|rule| rule.target.clone()).ok_or_else(
                    || {
                        ModelRegistryError::InvalidFallbackRule(
                            "target_logical requires target".to_owned(),
                        )
                    },
                )?)
            }
            AiccFallbackMode::TargetExact => {
                FallbackTarget::Exact(fallback.and_then(|rule| rule.target.clone()).ok_or_else(
                    || {
                        ModelRegistryError::InvalidFallbackRule(
                            "target_exact requires target".to_owned(),
                        )
                    },
                )?)
            }
        })
    }

    fn validate_item_graph(&self) -> Result<(), ModelRegistryError> {
        let mut complete = BTreeSet::new();
        for path in self.logical_nodes.keys() {
            self.visit_item_graph(path, &mut BTreeSet::new(), &mut complete)?;
        }
        Ok(())
    }

    fn disable_line(&self, path: &str) -> ModelDisable {
        self.logical_nodes
            .get(path)
            .map(|node| node.disable_line.clone())
            .unwrap_or_default()
    }

    fn default_options(&self, path: &str) -> BTreeMap<String, Value> {
        self.logical_nodes
            .get(path)
            .and_then(|node| node.definition.as_ref())
            .map(|definition| definition.default_options.clone())
            .unwrap_or_default()
    }

    fn scheduler_profile(&self, path: &str) -> AiccSchedulerProfile {
        self.logical_nodes
            .get(path)
            .and_then(|node| node.definition.as_ref())
            .map(|definition| definition.scheduler_profile.clone())
            .unwrap_or(AiccSchedulerProfile::Balanced)
    }

    fn visit_item_graph(
        &self,
        path: &str,
        visiting: &mut BTreeSet<String>,
        complete: &mut BTreeSet<String>,
    ) -> Result<(), ModelRegistryError> {
        if complete.contains(path) {
            return Ok(());
        }
        if !visiting.insert(path.to_owned()) {
            return Err(ModelRegistryError::LogicalTreeLoop(path.to_owned()));
        }
        if let Some(node) = self.logical_nodes.get(path) {
            for item in node.items.values() {
                let target = &item.item.target;
                if !target.contains('@') && self.logical_nodes.contains_key(target) {
                    ensure_same_namespace(path, target)?;
                    self.visit_item_graph(target, visiting, complete)?;
                }
            }
        }
        visiting.remove(path);
        complete.insert(path.to_owned());
        Ok(())
    }

    fn validate_fallback_graph(&self) -> Result<(), ModelRegistryError> {
        for start in self.logical_nodes.keys() {
            let mut current = start.clone();
            let mut visited = BTreeSet::new();
            for depth in 0..=self.fallback_depth_limit {
                if !visited.insert(current.clone()) {
                    return Err(ModelRegistryError::FallbackLoop(current));
                }
                let Some(rule) = self
                    .logical_nodes
                    .get(&current)
                    .and_then(|node| node.fallback.as_ref())
                else {
                    break;
                };
                let next = match rule.mode {
                    AiccFallbackMode::Parent => parent_logical_path(&current),
                    AiccFallbackMode::TargetLogical => rule.target.clone(),
                    _ => None,
                };
                let Some(next) = next else {
                    break;
                };
                ensure_same_namespace(&current, &next)?;
                if depth == self.fallback_depth_limit {
                    return Err(ModelRegistryError::FallbackDepthExceeded(
                        self.fallback_depth_limit,
                    ));
                }
                current = next;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FallbackTarget {
    None,
    Logical(String),
    Exact(String),
}

impl From<&RegisteredModel> for ModelView {
    fn from(model: &RegisteredModel) -> Self {
        Self {
            exact_model: model.exact_model.to_string(),
            model_uid: model.model_uid.to_string(),
            provider_instance_name: model.identity.provider_instance_name.clone(),
            provider_profile_id: model.identity.provider_profile_id.clone(),
            protocol_adapter_id: model.identity.protocol_adapter_id.clone(),
            model_driver_id: model.identity.model_driver_id.clone(),
            origin_model_id: model.identity.origin_model_id.clone(),
            provider_model_id: model.identity.provider_model_id.clone(),
            variant: model.exact_model.variant.clone(),
            api_types: model
                .api_types
                .iter()
                .map(|api_type| api_type_name(*api_type).to_owned())
                .collect(),
            logical_mounts: model.logical_mounts.clone(),
            capabilities: model.capabilities.clone(),
            attributes: model.attributes.clone(),
            operations: model.operations.clone(),
            inventory_revision: model.inventory_revision.clone(),
        }
    }
}

fn validate_identity(
    field: &'static str,
    value: &str,
    allow_colon: bool,
) -> Result<(), ModelRegistryError> {
    validate_non_empty(field, value)?;
    if value.contains('@') || (!allow_colon && value.contains(':')) {
        return Err(ModelRegistryError::InvalidIdentity {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), ModelRegistryError> {
    if value.is_empty() || value.trim() != value {
        Err(ModelRegistryError::InvalidIdentity {
            field,
            value: value.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn validate_variant(value: &str) -> Result<(), ModelRegistryError> {
    if value.is_empty() || value.trim() != value || value.contains(['@', ':']) {
        Err(ModelRegistryError::InvalidVariant(value.to_owned()))
    } else {
        Ok(())
    }
}

fn encode_uid_component(value: &str) -> String {
    value.replace('%', "%25").replace(':', "%3A")
}

fn deduplicate_api_types(api_types: &[ApiType]) -> Vec<ApiType> {
    let mut result = api_types.to_vec();
    result.sort_by_key(|api_type| api_type_name(*api_type));
    result.dedup();
    result
}

fn api_type_name(api_type: ApiType) -> &'static str {
    match api_type {
        ApiType::Llm => "llm",
        ApiType::EmbeddingText => "embedding.text",
        ApiType::EmbeddingMultimodal => "embedding.multimodal",
        ApiType::Rerank => "rerank",
        ApiType::ImageTextToImage => "image.txt2img",
        ApiType::ImageImageToImage => "image.img2img",
        ApiType::ImageInpaint => "image.inpaint",
        ApiType::ImageUpscale => "image.upscale",
        ApiType::ImageBackgroundRemove => "image.bg_remove",
        ApiType::VisionOcr => "vision.ocr",
        ApiType::VisionCaption => "vision.caption",
        ApiType::VisionDetect => "vision.detect",
        ApiType::VisionSegment => "vision.segment",
        ApiType::AudioTextToSpeech => "audio.tts",
        ApiType::AudioSpeechRecognition => "audio.asr",
        ApiType::AudioMusic => "audio.music",
        ApiType::AudioEnhance => "audio.enhance",
        ApiType::VideoTextToVideo => "video.txt2video",
        ApiType::VideoImageToVideo => "video.img2video",
        ApiType::VideoToVideo => "video.video2video",
        ApiType::VideoExtend => "video.extend",
        ApiType::VideoUpscale => "video.upscale",
        ApiType::AgentComputerUse => "agent.computer_use",
    }
}

fn path_namespace(path: &str) -> &str {
    path.split('.').next().unwrap_or_default()
}

fn path_matches_api_namespace(path: &str, api_type: ApiType) -> bool {
    path_namespace(path) == api_namespace(api_type)
}

fn api_namespace(api_type: ApiType) -> &'static str {
    match api_type {
        ApiType::AgentComputerUse => "agent_runtime",
        _ => api_type_name(api_type)
            .split('.')
            .next()
            .unwrap_or_default(),
    }
}

fn ensure_path_api_namespace(path: &str, api_type: ApiType) -> Result<(), ModelRegistryError> {
    if path_matches_api_namespace(path, api_type) {
        Ok(())
    } else {
        Err(ModelRegistryError::ApiNamespaceMismatch {
            path: path.to_owned(),
            api_type: api_type_name(api_type).to_owned(),
        })
    }
}

fn ensure_same_namespace(from: &str, to: &str) -> Result<(), ModelRegistryError> {
    validate_logical_path(to)?;
    if path_namespace(from) == path_namespace(to) {
        Ok(())
    } else {
        Err(ModelRegistryError::CrossNamespaceLink {
            from: from.to_owned(),
            to: to.to_owned(),
        })
    }
}

fn validate_logical_path(path: &str) -> Result<(), ModelRegistryError> {
    if path.is_empty()
        || path.trim() != path
        || path.contains('@')
        || path.split('.').any(str::is_empty)
        || path.chars().any(char::is_whitespace)
    {
        Err(ModelRegistryError::InvalidLogicalPath(path.to_owned()))
    } else {
        Ok(())
    }
}

fn parent_logical_path(path: &str) -> Option<String> {
    path.rsplit_once('.').map(|(parent, _)| parent.to_owned())
}

fn disabled_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Disabled,
        target: None,
    })
}

fn effective_items(
    items: &BTreeMap<String, ModelItem>,
    source: LogicalItemSource,
) -> BTreeMap<String, EffectiveItem> {
    items
        .iter()
        .map(|(name, item)| {
            (
                name.clone(),
                EffectiveItem {
                    item: item.clone(),
                    source,
                },
            )
        })
        .collect()
}

fn validate_weight(weight: f64, field: String) -> Result<(), ModelRegistryError> {
    if !weight.is_finite() || weight < 0.0 {
        Err(ModelRegistryError::InvalidWeight { field, weight })
    } else {
        Ok(())
    }
}

fn validate_weight_map(
    weights: &BTreeMap<String, f64>,
    field: &str,
) -> Result<(), ModelRegistryError> {
    for (name, weight) in weights {
        validate_weight(*weight, format!("{field}.{name}"))?;
    }
    Ok(())
}

fn validate_items(
    path: &str,
    items: &BTreeMap<String, ModelItem>,
) -> Result<(), ModelRegistryError> {
    for (name, item) in items {
        if name.is_empty() {
            return Err(ModelRegistryError::InvalidItemName(path.to_owned()));
        }
        validate_weight(item.weight, format!("{path}.items.{name}.weight"))?;
        if item.target.contains('@') {
            ExactModelName::parse(&item.target)?;
        } else {
            ensure_same_namespace(path, &item.target)?;
        }
    }
    Ok(())
}

fn apply_item_patches(
    path: &str,
    items: &mut BTreeMap<String, EffectiveItem>,
    patches: &BTreeMap<String, ModelItemPatch>,
    source: LogicalItemSource,
) -> Result<(), ModelRegistryError> {
    for (name, patch) in patches {
        if let Some(weight) = patch.weight {
            validate_weight(weight, format!("{path}.item_overrides.{name}.weight"))?;
        }
        if let Some(target) = &patch.target {
            if target.contains('@') {
                ExactModelName::parse(target)?;
            } else {
                ensure_same_namespace(path, target)?;
            }
        }
        if let Some(item) = items.get_mut(name) {
            if let Some(target) = &patch.target {
                item.item.target = target.clone();
            }
            if let Some(weight) = patch.weight {
                item.item.weight = weight;
            }
            item.source = source;
        } else {
            let target =
                patch
                    .target
                    .clone()
                    .ok_or_else(|| ModelRegistryError::UnknownItemOverride {
                        path: path.to_owned(),
                        item: name.clone(),
                    })?;
            items.insert(
                name.clone(),
                EffectiveItem {
                    item: ModelItem::new(target, patch.weight.unwrap_or(1.0)),
                    source,
                },
            );
        }
    }
    Ok(())
}

fn flatten_logical_tree(
    tree: &BTreeMap<String, AiccLogicalNodeOverlay>,
) -> Result<Vec<(String, &AiccLogicalNodeOverlay)>, ModelRegistryError> {
    fn visit<'a>(
        tree: &'a BTreeMap<String, AiccLogicalNodeOverlay>,
        parent: Option<&str>,
        seen: &mut BTreeSet<String>,
        result: &mut Vec<(String, &'a AiccLogicalNodeOverlay)>,
    ) -> Result<(), ModelRegistryError> {
        for (name, node) in tree {
            let path = parent.map_or_else(|| name.clone(), |parent| format!("{parent}.{name}"));
            validate_logical_path(&path)?;
            if !seen.insert(path.clone()) {
                return Err(ModelRegistryError::DuplicateOverlayPath(path));
            }
            result.push((path.clone(), node));
            visit(&node.children, Some(&path), seen, result)?;
        }
        Ok(())
    }
    let mut result = Vec::new();
    visit(tree, None, &mut BTreeSet::new(), &mut result)?;
    Ok(result)
}

fn validate_fallback_rule(rule: Option<&AiccFallbackRule>) -> Result<(), ModelRegistryError> {
    let Some(rule) = rule else {
        return Ok(());
    };
    match rule.mode {
        AiccFallbackMode::TargetExact => {
            ExactModelName::parse(rule.target.as_deref().ok_or_else(|| {
                ModelRegistryError::InvalidFallbackRule("target_exact requires target".to_owned())
            })?)?;
        }
        AiccFallbackMode::TargetLogical => {
            validate_logical_path(rule.target.as_deref().ok_or_else(|| {
                ModelRegistryError::InvalidFallbackRule("target_logical requires target".to_owned())
            })?)?;
        }
        _ if rule.target.is_some() => {
            return Err(ModelRegistryError::InvalidFallbackRule(
                "only target_exact and target_logical accept target".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn missing_requirements(requirement: &ModelRequirement, model: &RegisteredModel) -> Vec<String> {
    let mut missing = Vec::new();
    for (required, name) in [
        (requirement.streaming, "streaming"),
        (requirement.tool_call, "tool_call"),
        (requirement.json_schema, "json_schema"),
        (requirement.web_search, "web_search"),
        (requirement.vision, "vision"),
        (requirement.image_generation, "image_generation"),
    ] {
        let supported = model
            .capabilities
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if required && !supported {
            missing.push(name.to_owned());
        }
    }
    if let Some(required) = requirement.min_context_tokens {
        let available = model
            .capabilities
            .get("max_context_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if available < required {
            missing.push(format!("min_context_tokens:{required}"));
        }
    }
    missing
}

fn normalize_admissions(admissions: &mut Vec<AdmissionRecord>) {
    admissions.sort_by(|left, right| {
        (&left.logical_path, &left.exact_model).cmp(&(&right.logical_path, &right.exact_model))
    });
    admissions.dedup_by(|left, right| {
        left.logical_path == right.logical_path && left.exact_model == right.exact_model
    });
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ModelRegistryError {
    InvalidIdentity {
        field: &'static str,
        value: String,
    },
    InvalidExactModelName(String),
    InvalidVariant(String),
    InvalidLogicalPath(String),
    ApiNamespaceMismatch {
        path: String,
        api_type: String,
    },
    MountApiMismatch {
        path: String,
        provider_model_id: String,
    },
    CrossNamespaceLink {
        from: String,
        to: String,
    },
    DuplicateProviderInstance(String),
    DuplicateExactModel(String),
    DuplicateVariant {
        provider_model_id: String,
        variant: String,
    },
    UnknownModelDriver(String),
    MissingApiTypes(String),
    DuplicateLogicalDefinition(String),
    DuplicateOverlayPath(String),
    InvalidItemName(String),
    InvalidWeight {
        field: String,
        weight: f64,
    },
    ItemsAndOverridesConflict(String),
    UnknownItemOverride {
        path: String,
        item: String,
    },
    UnknownLogicalProfile(String),
    InvalidFallbackRule(String),
    LogicalTreeLoop(String),
    FallbackLoop(String),
    FallbackDepthExceeded(usize),
}

impl fmt::Display for ModelRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity { field, value } => {
                write!(formatter, "invalid {field} `{value}`")
            }
            Self::InvalidExactModelName(value) => {
                write!(formatter, "invalid exact model `{value}`")
            }
            Self::InvalidVariant(value) => write!(formatter, "invalid variant `{value}`"),
            Self::InvalidLogicalPath(path) => write!(formatter, "invalid logical path `{path}`"),
            Self::ApiNamespaceMismatch { path, api_type } => {
                write!(
                    formatter,
                    "logical path `{path}` does not match `{api_type}`"
                )
            }
            Self::MountApiMismatch {
                path,
                provider_model_id,
            } => write!(
                formatter,
                "logical mount `{path}` does not match model `{provider_model_id}` API types"
            ),
            Self::CrossNamespaceLink { from, to } => {
                write!(
                    formatter,
                    "logical link crosses namespace from `{from}` to `{to}`"
                )
            }
            Self::DuplicateProviderInstance(value) => {
                write!(formatter, "duplicate provider `{value}`")
            }
            Self::DuplicateExactModel(value) => {
                write!(formatter, "duplicate exact model `{value}`")
            }
            Self::DuplicateVariant {
                provider_model_id,
                variant,
            } => {
                write!(
                    formatter,
                    "duplicate variant `{variant}` for `{provider_model_id}`"
                )
            }
            Self::UnknownModelDriver(value) => write!(formatter, "unknown model driver `{value}`"),
            Self::MissingApiTypes(value) => write!(formatter, "model `{value}` has no API types"),
            Self::DuplicateLogicalDefinition(path) => {
                write!(formatter, "duplicate definition `{path}`")
            }
            Self::DuplicateOverlayPath(path) => {
                write!(formatter, "duplicate overlay path `{path}`")
            }
            Self::InvalidItemName(path) => write!(formatter, "empty item name at `{path}`"),
            Self::InvalidWeight { field, weight } => {
                write!(formatter, "invalid weight `{weight}` at `{field}`")
            }
            Self::ItemsAndOverridesConflict(path) => {
                write!(formatter, "`{path}` has both items and item_overrides")
            }
            Self::UnknownItemOverride { path, item } => {
                write!(formatter, "`{path}` overrides unknown item `{item}`")
            }
            Self::UnknownLogicalProfile(profile) => {
                write!(formatter, "unknown profile `{profile}`")
            }
            Self::InvalidFallbackRule(reason) => write!(formatter, "invalid fallback: {reason}"),
            Self::LogicalTreeLoop(path) => write!(formatter, "logical tree loop at `{path}`"),
            Self::FallbackLoop(path) => write!(formatter, "fallback loop at `{path}`"),
            Self::FallbackDepthExceeded(limit) => {
                write!(formatter, "fallback depth exceeds {limit}")
            }
        }
    }
}

impl Error for ModelRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogDocuments, ModelDriverCatalog};
    use buckyos_api::{AiccSessionLogicalProfile, LogicalItems};
    use serde_json::json;

    fn catalog() -> CatalogSnapshot {
        let driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "openai",
            "revision_seq": 1,
            "models": [],
            "patterns": [{
                "match": "*",
                "api_types": ["llm"],
                "capabilities": {"streaming": true}
            }],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![driver],
                ..CatalogDocuments::default()
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap()
    }

    fn inventory_model(id: &str, tool_call: bool) -> InventoryModel {
        InventoryModel {
            provider_model_id: id.to_owned(),
            model_driver_id: "openai".to_owned(),
            origin_model_id: id.to_owned(),
            api_types: vec![ApiType::Llm],
            logical_mounts: vec!["llm.gpt".to_owned()],
            variants: Vec::new(),
            capabilities: BTreeMap::from([
                ("streaming".to_owned(), json!(true)),
                ("tool_call".to_owned(), json!(tool_call)),
                ("max_context_tokens".to_owned(), json!(128_000)),
            ]),
            attributes: BTreeMap::new(),
            operations: BTreeMap::from([(
                "chat.completions.create".to_owned(),
                "responses.create".to_owned(),
            )]),
        }
    }

    fn inventory(instance: &str, models: Vec<InventoryModel>) -> ProviderInventory {
        ProviderInventory {
            provider_instance_name: instance.to_owned(),
            provider_profile_id: "openai".to_owned(),
            protocol_adapter_id: "openai-responses".to_owned(),
            inventory_revision: format!("inv-{instance}"),
            models,
        }
    }

    fn definition(path: &str, tool_call: bool, mode: MountMode) -> LogicalModelDefinition {
        LogicalModelDefinition {
            path: path.to_owned(),
            api_type: ApiType::Llm,
            min_line: ModelRequirement {
                tool_call,
                ..ModelRequirement::default()
            },
            disable_line: ModelDisable::default(),
            default_options: BTreeMap::new(),
            mount_mode: mode,
            scheduler_profile: AiccSchedulerProfile::Balanced,
            fallback: Some(AiccFallbackRule {
                mode: AiccFallbackMode::Strict,
                target: None,
            }),
            route_policy: AiccPolicyConfig::default(),
            user_visible_tier: None,
        }
    }

    fn item_node(entries: &[(&str, &str, f64)]) -> AiccLogicalNodeOverlay {
        AiccLogicalNodeOverlay {
            items: Some(
                entries
                    .iter()
                    .map(|(name, target, weight)| {
                        ((*name).to_owned(), ModelItem::new(*target, *weight))
                    })
                    .collect(),
            ),
            ..AiccLogicalNodeOverlay::default()
        }
    }

    fn layer(path: &str, entries: &[(&str, &str, f64)]) -> AiccRouteOverlay {
        AiccRouteOverlay {
            logical_tree: BTreeMap::from([(path.to_owned(), item_node(entries))]),
            ..AiccRouteOverlay::default()
        }
    }

    #[test]
    fn exact_model_and_uid_preserve_all_identity_dimensions() {
        let exact = ExactModelName::new(
            "gpt-5.2",
            Some("reasoning-high".to_owned()),
            "openai_primary",
        )
        .unwrap();
        assert_eq!(exact.as_str(), "gpt-5.2:reasoning-high@openai_primary");
        assert_eq!(exact.provider_model_id(), "gpt-5.2");
        assert_eq!(exact.variant(), Some("reasoning-high"));
        assert_eq!(exact.provider_instance_name(), "openai_primary");
        assert_eq!(ExactModelName::parse(exact.as_str()).unwrap(), exact);
        assert!(ExactModelName::parse("gpt@one@two").is_err());
        assert!(ExactModelName::new("gpt:ambiguous", None, "one").is_err());

        let uid = ModelUid::new(
            "openai",
            "gpt:5.2",
            "openai-responses",
            Some("reasoning-high".to_owned()),
        )
        .unwrap();
        assert_eq!(
            uid.to_string(),
            "openai:gpt%3A5.2:openai-responses:reasoning-high"
        );
    }

    #[test]
    fn fixture_inventory_builds_base_variant_and_read_only_views() {
        let mut model = inventory_model("gpt-5.2", true);
        model.variants.push(InventoryModelVariant {
            name: "reasoning-high".to_owned(),
            logical_mounts: vec!["llm.reason".to_owned()],
        });
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![model])],
            vec![
                definition("llm.gpt", false, MountMode::Manual),
                definition("llm.reason", true, MountMode::Manual),
            ],
            RegistryLayers::default(),
        )
        .unwrap();

        assert_eq!(registry.model_views().len(), 2);
        let variant = registry
            .exact_model("gpt-5.2:reasoning-high@primary")
            .unwrap();
        assert_eq!(variant.identity.provider_profile_id, "openai");
        assert_eq!(variant.identity.protocol_adapter_id, "openai-responses");
        assert_eq!(variant.identity.model_driver_id, "openai");
        assert_eq!(variant.identity.origin_model_id, "gpt-5.2");
        assert_eq!(variant.identity.provider_model_id, "gpt-5.2");
        assert_eq!(
            registry
                .resolve_candidates("llm.reason", ApiType::Llm)
                .unwrap()
                .candidates
                .len(),
            1
        );
    }

    #[test]
    fn inventory_conflicts_and_unknown_drivers_are_rejected() {
        let duplicate_instances = vec![
            inventory("same", vec![inventory_model("a", true)]),
            inventory("same", vec![inventory_model("b", true)]),
        ];
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &duplicate_instances,
                Vec::new(),
                RegistryLayers::default()
            ),
            Err(ModelRegistryError::DuplicateProviderInstance(_))
        ));

        let mut unknown = inventory_model("a", true);
        unknown.model_driver_id = "missing".to_owned();
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[inventory("one", vec![unknown])],
                Vec::new(),
                RegistryLayers::default()
            ),
            Err(ModelRegistryError::UnknownModelDriver(_))
        ));
    }

    #[test]
    fn auto_mount_enforces_min_line_and_reports_rejections() {
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory(
                "primary",
                vec![
                    inventory_model("capable", true),
                    inventory_model("basic", false),
                ],
            )],
            vec![definition("llm.plan", true, MountMode::Auto)],
            RegistryLayers::default(),
        )
        .unwrap();
        let result = registry
            .resolve_candidates("llm.plan", ApiType::Llm)
            .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].model.identity.provider_model_id,
            "capable"
        );
        assert_eq!(
            result.candidates[0].paths[0].sources,
            vec![LogicalItemSource::AutoAdmission]
        );
        let rejected = result
            .admissions
            .iter()
            .find(|record| record.exact_model == "basic@primary")
            .unwrap();
        assert!(!rejected.admitted);
        assert_eq!(rejected.missing_requirements, vec!["tool_call"]);
    }

    #[test]
    fn links_deduplicate_exact_models_and_preserve_each_source_path() {
        let overlay = AiccRouteOverlay {
            logical_tree: BTreeMap::from([
                (
                    "llm.plan".to_owned(),
                    item_node(&[("a", "llm.family_a", 2.0), ("b", "llm.family_b", 1.0)]),
                ),
                (
                    "llm.family_a".to_owned(),
                    item_node(&[("model", "gpt@primary", 1.0)]),
                ),
                (
                    "llm.family_b".to_owned(),
                    item_node(&[("model", "gpt@primary", 1.0)]),
                ),
            ]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", true)])],
            vec![definition("llm.plan", false, MountMode::Manual)],
            RegistryLayers {
                system: Some(&overlay),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("llm.plan", ApiType::Llm)
            .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].paths.len(), 2);
        assert_eq!(result.candidates[0].paths[0].priority, vec![2.0, 1.0]);
        assert_eq!(result.candidates[0].paths[1].priority, vec![1.0, 1.0]);
    }

    #[test]
    fn factory_user_and_session_overlays_compose_in_order() {
        let factory = layer("llm.chat", &[("primary", "gpt@primary", 1.0)]);
        let user = AiccRouteOverlay {
            logical_profile: Some(AiccSessionLogicalProfile {
                overlays: vec![AiccLogicalTreeOverlay {
                    path: "llm.chat".to_owned(),
                    item_overrides: BTreeMap::from([(
                        "primary".to_owned(),
                        ModelItemPatch {
                            weight: Some(2.0),
                            ..ModelItemPatch::default()
                        },
                    )]),
                    ..AiccLogicalTreeOverlay::default()
                }],
                ..AiccSessionLogicalProfile::default()
            }),
            ..AiccRouteOverlay::default()
        };
        let session = AiccRouteOverlay {
            provider_weights: BTreeMap::from([("backup".to_owned(), 0.25)]),
            logical_profile: Some(AiccSessionLogicalProfile {
                overlays: vec![AiccLogicalTreeOverlay {
                    path: "llm.chat".to_owned(),
                    merge_mode: OverlayMergeMode::Replace,
                    items: LogicalItems::from([(
                        "only".to_owned(),
                        ModelItem::new("mini@backup", 4.0),
                    )]),
                    ..AiccLogicalTreeOverlay::default()
                }],
                ..AiccSessionLogicalProfile::default()
            }),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[
                inventory("primary", vec![inventory_model("gpt", true)]),
                inventory("backup", vec![inventory_model("mini", true)]),
            ],
            vec![definition("llm.chat", false, MountMode::Manual)],
            RegistryLayers {
                factory: Some(&factory),
                user: Some(&user),
                session: Some(&session),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("llm.chat", ApiType::Llm)
            .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].model.exact_model.as_str(),
            "mini@backup"
        );
        assert_eq!(result.candidates[0].provider_weight, 0.25);
        assert_eq!(
            result.candidates[0].paths[0].sources,
            vec![LogicalItemSource::SessionOverlay]
        );
        let view = registry
            .logical_model_views()
            .into_iter()
            .find(|view| view.path == "llm.chat")
            .unwrap();
        assert_eq!(view.fallback.unwrap().mode, AiccFallbackMode::Disabled);
    }

    #[test]
    fn invalid_overlay_forms_and_weights_are_rejected() {
        let conflict = AiccRouteOverlay {
            logical_tree: BTreeMap::from([(
                "llm.chat".to_owned(),
                AiccLogicalNodeOverlay {
                    items: Some(LogicalItems::new()),
                    item_overrides: Some(BTreeMap::new()),
                    ..AiccLogicalNodeOverlay::default()
                },
            )]),
            ..AiccRouteOverlay::default()
        };
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    session: Some(&conflict),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::ItemsAndOverridesConflict(_))
        ));

        let invalid = layer("llm.chat", &[("bad", "gpt@primary", -1.0)]);
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&invalid),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::InvalidWeight { .. })
        ));
    }

    #[test]
    fn logical_cycles_and_cross_namespace_links_are_rejected() {
        let cycle = AiccRouteOverlay {
            logical_tree: BTreeMap::from([
                ("llm.a".to_owned(), item_node(&[("b", "llm.b", 1.0)])),
                ("llm.b".to_owned(), item_node(&[("a", "llm.a", 1.0)])),
            ]),
            ..AiccRouteOverlay::default()
        };
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&cycle),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::LogicalTreeLoop(_))
        ));

        let cross = layer("llm.chat", &[("bad", "image.txt2img", 1.0)]);
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&cross),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::CrossNamespaceLink { .. })
        ));
    }

    #[test]
    fn parent_and_exact_fallbacks_resolve_deterministically() {
        let factory = layer("llm", &[("model", "gpt@primary", 1.0)]);
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", false)])],
            vec![LogicalModelDefinition {
                fallback: Some(AiccFallbackRule {
                    mode: AiccFallbackMode::Parent,
                    target: None,
                }),
                ..definition("llm.code", true, MountMode::Auto)
            }],
            RegistryLayers {
                factory: Some(&factory),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("llm.code", ApiType::Llm)
            .unwrap();
        assert_eq!(result.resolved_logical_path, "llm");
        assert_eq!(result.fallback_chain.len(), 1);
        assert_eq!(
            result.candidates[0].model.exact_model.as_str(),
            "gpt@primary"
        );
        assert!(result.admissions.iter().any(|record| {
            record.logical_path == "llm.code"
                && record.exact_model == "gpt@primary"
                && !record.admitted
        }));

        let exact_definition = LogicalModelDefinition {
            fallback: Some(AiccFallbackRule {
                mode: AiccFallbackMode::TargetExact,
                target: Some("gpt@primary".to_owned()),
            }),
            ..definition("llm.strict", false, MountMode::Manual)
        };
        let exact_registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", true)])],
            vec![exact_definition],
            RegistryLayers::default(),
        )
        .unwrap();
        let exact = exact_registry
            .resolve_candidates("llm.strict", ApiType::Llm)
            .unwrap();
        assert_eq!(exact.resolved_logical_path, "gpt@primary");
        assert_eq!(exact.candidates.len(), 1);
    }

    #[test]
    fn fallback_loops_and_excess_depth_are_rejected() {
        let mut loop_overlay = AiccRouteOverlay::default();
        for (path, target) in [("llm.a", "llm.b"), ("llm.b", "llm.a")] {
            loop_overlay.logical_tree.insert(
                path.to_owned(),
                AiccLogicalNodeOverlay {
                    fallback: Some(AiccFallbackRule {
                        mode: AiccFallbackMode::TargetLogical,
                        target: Some(target.to_owned()),
                    }),
                    ..AiccLogicalNodeOverlay::default()
                },
            );
        }
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&loop_overlay),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::FallbackLoop(_))
        ));

        let mut deep = AiccRouteOverlay::default();
        for index in 0..=DEFAULT_FALLBACK_DEPTH_LIMIT {
            deep.logical_tree.insert(
                format!("llm.d{index}"),
                AiccLogicalNodeOverlay {
                    fallback: Some(AiccFallbackRule {
                        mode: AiccFallbackMode::TargetLogical,
                        target: Some(format!("llm.d{}", index + 1)),
                    }),
                    ..AiccLogicalNodeOverlay::default()
                },
            );
        }
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&deep),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::FallbackDepthExceeded(_))
        ));
    }

    #[test]
    fn exact_model_lookup_never_guesses_provider_or_capability_from_names() {
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory(
                "arbitrary_instance",
                vec![inventory_model("unrecognizable-model", true)],
            )],
            vec![definition("llm.chat", true, MountMode::Auto)],
            RegistryLayers::default(),
        )
        .unwrap();
        let model = registry
            .exact_model("unrecognizable-model@arbitrary_instance")
            .unwrap();
        assert_eq!(model.identity.provider_profile_id, "openai");
        assert_eq!(model.identity.model_driver_id, "openai");
        assert_eq!(model.api_types, vec![ApiType::Llm]);
    }

    #[test]
    fn api_namespaces_include_agent_runtime_and_reject_bad_inventory_mounts() {
        let mut agent = inventory_model("computer", false);
        agent.api_types = vec![ApiType::AgentComputerUse];
        agent.logical_mounts = vec!["agent_runtime.computer_use".to_owned()];
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("agent", vec![agent])],
            vec![LogicalModelDefinition {
                path: "agent_runtime.computer_use".to_owned(),
                api_type: ApiType::AgentComputerUse,
                min_line: ModelRequirement::default(),
                disable_line: ModelDisable::default(),
                default_options: BTreeMap::new(),
                mount_mode: MountMode::Manual,
                scheduler_profile: AiccSchedulerProfile::Balanced,
                fallback: None,
                route_policy: AiccPolicyConfig::default(),
                user_visible_tier: None,
            }],
            RegistryLayers::default(),
        )
        .unwrap();
        assert_eq!(
            registry
                .resolve_candidates("agent_runtime.computer_use", ApiType::AgentComputerUse)
                .unwrap()
                .candidates
                .len(),
            1
        );

        let mut invalid = inventory_model("bad", false);
        invalid.logical_mounts = vec!["image.txt2img".to_owned()];
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[inventory("bad", vec![invalid])],
                Vec::new(),
                RegistryLayers::default()
            ),
            Err(ModelRegistryError::MountApiMismatch { .. })
        ));
    }
}
