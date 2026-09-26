use crate::canonical::CanonicalFieldMapping;
use crate::catalog::{CatalogSnapshot, LlmModel, ModelStability};
use crate::error::ModelRegistryError;
use buckyos_api::{
    AiccFallbackMode, AiccFallbackRule, AiccLogicalDirectoryKind, AiccLogicalNodeOverlay,
    AiccLogicalTreeOverlay, AiccPolicyConfig, AiccRouteOverlay, AiccRoutingCommand,
    AiccRoutingCommandStatus, AiccSchedulerProfile, ApiType, LogicalItem, ModelDisable,
    ModelItemPatch, ModelRequirement, OverlayMergeMode,
};
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    pub canonical_fields: BTreeMap<String, CanonicalFieldMapping>,
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
    RoutingCommand,
}

pub(crate) fn logical_item_source_name(source: LogicalItemSource) -> &'static str {
    match source {
        LogicalItemSource::BuiltinDefinition => "builtin_definition",
        LogicalItemSource::DriverMetadataMount => "driver_metadata_mount",
        LogicalItemSource::AutoAdmission => "auto_admission",
        LogicalItemSource::ManualOverride => "manual_override",
        LogicalItemSource::UserOverlay => "user_overlay",
        LogicalItemSource::SessionOverlay => "session_overlay",
        LogicalItemSource::RoutingCommand => "routing_command",
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EffectiveItem {
    name: String,
    target: String,
    weight: f64,
    default_weight: f64,
    source: LogicalItemSource,
    weight_source: LogicalItemSource,
}

impl EffectiveItem {
    fn new(
        name: impl Into<String>,
        target: impl Into<String>,
        weight: f64,
        source: LogicalItemSource,
    ) -> Self {
        Self {
            name: name.into(),
            target: target.into(),
            weight,
            default_weight: weight,
            source,
            weight_source: source,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EffectiveLogicalNode {
    definition: Option<LogicalModelDefinition>,
    family: Option<LlmModel>,
    family_members: BTreeSet<String>,
    items: Vec<EffectiveItem>,
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
    pub canonical_fields: BTreeMap<String, CanonicalFieldMapping>,
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
    pub canonical_fields: BTreeMap<String, CanonicalFieldMapping>,
    pub attributes: BTreeMap<String, Value>,
    pub operations: BTreeMap<String, String>,
    pub inventory_revision: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogicalItemView {
    pub name: String,
    pub target: String,
    pub weight: f64,
    pub default_weight: f64,
    pub source: LogicalItemSource,
    pub weight_source: LogicalItemSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogicalModelView {
    pub path: String,
    pub kind: AiccLogicalDirectoryKind,
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
    pub weights: Vec<f64>,
    pub sources: Vec<LogicalItemSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegistryCandidate {
    pub model: RegisteredModel,
    pub paths: Vec<CandidatePath>,
    pub experimental: bool,
    pub exact_model_weight: f64,
    pub provider_weight: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpansionState {
    Expanded,
    Unavailable,
    NotExpanded,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExpansionItem {
    pub name: String,
    pub target: String,
    pub weight: f64,
    pub weight_source: LogicalItemSource,
    pub state: ExpansionState,
}

struct ExpandContext<'a> {
    resolved_path: &'a str,
    accept: &'a mut dyn FnMut(&str, &RegistryCandidate) -> bool,
    explore_all: bool,
    admissions: Vec<AdmissionRecord>,
    expansions: Vec<ExpansionStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExpansionStep {
    pub logical_path: String,
    pub max_weight: Option<f64>,
    pub items: Vec<ExpansionItem>,
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
    pub expansions: Vec<ExpansionStep>,
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
    specs: BTreeMap<String, (String, bool)>,
    family_names: BTreeSet<String>,
    logical_nodes: BTreeMap<String, EffectiveLogicalNode>,
    global_exact_model_weights: BTreeMap<String, f64>,
    provider_weights: BTreeMap<String, f64>,
    fallback_depth_limit: usize,
    known_models: BTreeSet<(String, String)>,
    command_status: Vec<AiccRoutingCommandStatus>,
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
            specs: BTreeMap::new(),
            family_names: catalog
                .llm_models()
                .map(|(_, _, model)| model.family.clone())
                .collect(),
            logical_nodes: BTreeMap::new(),
            global_exact_model_weights: BTreeMap::new(),
            provider_weights: BTreeMap::new(),
            fallback_depth_limit: DEFAULT_FALLBACK_DEPTH_LIMIT,
            known_models: catalog
                .model_drivers()
                .flat_map(|driver| {
                    driver
                        .models
                        .iter()
                        .map(|model| (driver.model_driver_id.clone(), model.id.clone()))
                })
                .collect(),
            command_status: Vec::new(),
        };
        registry.register_definitions(definitions)?;
        registry.register_specs(catalog)?;
        registry.register_inventories(catalog, inventories)?;
        registry.materialize_families(catalog);
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
        let commands = [layers.factory, layers.system, layers.user, layers.session]
            .into_iter()
            .flatten()
            .flat_map(|layer| layer.routing_commands.iter().cloned())
            .collect::<Vec<_>>();
        registry.apply_routing_commands(&commands);
        registry.validate_llm_tree()?;
        registry.validate_fallback_graph()?;
        registry.validate_item_graph()?;
        Ok(registry)
    }

    pub(crate) fn with_session_overlay(
        &self,
        overlay: &AiccRouteOverlay,
    ) -> Result<Self, ModelRegistryError> {
        let mut registry = self.clone();
        registry.apply_route_overlay(overlay, LogicalItemSource::SessionOverlay)?;
        if !overlay.routing_commands.is_empty() {
            let mut status = registry.command_status.clone();
            registry.apply_routing_commands(&overlay.routing_commands);
            status.append(&mut registry.command_status);
            registry.command_status = status;
        }
        registry.validate_llm_tree()?;
        registry.validate_fallback_graph()?;
        registry.validate_item_graph()?;
        Ok(registry)
    }

    pub(crate) fn exact_model(&self, exact_model: &str) -> Option<&RegisteredModel> {
        self.models.get(exact_model)
    }

    pub(crate) fn logical_route_policy(&self, logical_path: &str) -> Option<&AiccPolicyConfig> {
        self.logical_nodes
            .get(logical_path)
            .and_then(|node| node.definition.as_ref())
            .map(|definition| &definition.route_policy)
    }

    pub(crate) fn model_views(&self) -> Vec<ModelView> {
        self.models.values().map(ModelView::from).collect()
    }

    pub(crate) fn logical_model_views(&self) -> Vec<LogicalModelView> {
        self.logical_nodes
            .iter()
            .filter(|(path, node)| !self.family_names.contains(*path) || node.family.is_some())
            .map(|(path, node)| LogicalModelView {
                path: path.clone(),
                kind: self.directory_kind(path, node),
                api_type: node
                    .definition
                    .as_ref()
                    .map(|definition| api_type_name(definition.api_type).to_owned())
                    .or_else(|| {
                        (self.specs.contains_key(path) || node.family.is_some())
                            .then(|| "llm".to_owned())
                    }),
                mount_mode: node
                    .definition
                    .as_ref()
                    .map(|definition| definition.mount_mode),
                item_count: node.items.len(),
                items: node
                    .items
                    .iter()
                    .map(|item| LogicalItemView {
                        name: item.name.clone(),
                        target: item.target.clone(),
                        weight: item.weight,
                        default_weight: item.default_weight,
                        source: item.source,
                        weight_source: item.weight_source,
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

    pub(crate) fn routing_command_status(&self) -> &[AiccRoutingCommandStatus] {
        &self.command_status
    }

    fn directory_kind(&self, path: &str, node: &EffectiveLogicalNode) -> AiccLogicalDirectoryKind {
        if self.specs.contains_key(path) {
            AiccLogicalDirectoryKind::Spec
        } else if node.family.is_some() {
            AiccLogicalDirectoryKind::Family
        } else if node.definition.is_some() {
            AiccLogicalDirectoryKind::Task
        } else {
            AiccLogicalDirectoryKind::Directory
        }
    }

    pub(crate) fn logical_directory_kind(&self, path: &str) -> Option<AiccLogicalDirectoryKind> {
        self.logical_nodes
            .get(path)
            .map(|node| self.directory_kind(path, node))
    }

    fn apply_routing_commands(&mut self, commands: &[AiccRoutingCommand]) {
        let mut leaves = BTreeMap::new();
        let mut factors = Vec::new();
        let mut weights = Vec::new();
        for command in commands {
            match command {
                AiccRoutingCommand::ItemWeight { .. } => weights.push(command),
                _ => factors.push(command),
            }
        }
        let mut status = Vec::new();
        for command in factors.into_iter().chain(weights) {
            let stale_reason = command
                .validate()
                .err()
                .or_else(|| self.stale_reason(command));
            let mut matched_items = 0;
            if stale_reason.is_none() {
                let targets = self.command_targets(command, &mut leaves);
                for (path, index) in targets {
                    let item = &mut self
                        .logical_nodes
                        .get_mut(&path)
                        .expect("command target node exists")
                        .items[index];
                    match command {
                        AiccRoutingCommand::ItemWeight { weight, .. } => item.weight = *weight,
                        _ => item.weight *= command.value(),
                    }
                    item.weight_source = LogicalItemSource::RoutingCommand;
                    matched_items += 1;
                }
            }
            status.push(AiccRoutingCommandStatus {
                command: command.clone(),
                matched_items,
                stale_reason,
            });
        }
        self.command_status = status;
    }

    fn stale_reason(&self, command: &AiccRoutingCommand) -> Option<String> {
        match command {
            AiccRoutingCommand::VendorFactor { vendor, .. } => {
                (!self.known_models.iter().any(|(driver, _)| driver == vendor))
                    .then(|| format!("vendor {vendor} is not in the model catalog"))
            }
            AiccRoutingCommand::SpecFactor { spec, .. } => (!self.specs.contains_key(spec))
                .then(|| format!("specification {spec} no longer exists")),
            AiccRoutingCommand::ModelFactor { vendor, model, .. } => {
                (!self.known_models.contains(&(vendor.clone(), model.clone())))
                    .then(|| format!("model {vendor}/{model} is not in the model catalog"))
            }
            AiccRoutingCommand::ItemWeight { path, item, .. } => match self.logical_nodes.get(path)
            {
                None => Some(format!("directory {path} no longer exists")),
                Some(node) if !node.items.iter().any(|entry| &entry.name == item) => {
                    Some(format!("item {item} no longer exists in {path}"))
                }
                Some(_) => None,
            },
        }
    }

    fn command_targets(
        &self,
        command: &AiccRoutingCommand,
        leaves: &mut BTreeMap<String, BTreeSet<(String, String)>>,
    ) -> Vec<(String, usize)> {
        let mut targets = Vec::new();
        for (path, node) in &self.logical_nodes {
            for (index, item) in node.items.iter().enumerate() {
                let base = target_base(&item.target);
                let matched = match command {
                    AiccRoutingCommand::ItemWeight {
                        path: p, item: i, ..
                    } => p == path && i == &item.name,
                    AiccRoutingCommand::SpecFactor { spec, .. } => spec == base,
                    AiccRoutingCommand::VendorFactor { vendor, .. } => {
                        self.specs
                            .get(base)
                            .is_some_and(|(driver, _)| driver == vendor)
                            || self.owned_by(base, leaves, |(driver, _)| driver == vendor)
                    }
                    AiccRoutingCommand::ModelFactor { vendor, model, .. } => {
                        self.owned_by(base, leaves, |(driver, origin)| {
                            driver == vendor && origin == model
                        })
                    }
                };
                if matched {
                    targets.push((path.clone(), index));
                }
            }
        }
        targets
    }

    fn owned_by(
        &self,
        target: &str,
        leaves: &mut BTreeMap<String, BTreeSet<(String, String)>>,
        owner: impl Fn(&(String, String)) -> bool,
    ) -> bool {
        let members = self.target_leaves(target, leaves, &mut BTreeSet::new());
        !members.is_empty() && members.iter().all(owner)
    }

    fn target_leaves(
        &self,
        target: &str,
        leaves: &mut BTreeMap<String, BTreeSet<(String, String)>>,
        stack: &mut BTreeSet<String>,
    ) -> BTreeSet<(String, String)> {
        if let Some(model) = self.models.get(target) {
            return BTreeSet::from([(
                model.identity.model_driver_id.clone(),
                model.identity.origin_model_id.clone(),
            )]);
        }
        if let Some(cached) = leaves.get(target) {
            return cached.clone();
        }
        let mut members = BTreeSet::new();
        if let Some(node) = self.logical_nodes.get(target) {
            if stack.insert(target.to_owned()) {
                for item in &node.items {
                    let base = target_base(&item.target);
                    members.extend(self.target_leaves(base, leaves, stack));
                }
                stack.remove(target);
            }
        }
        leaves.insert(target.to_owned(), members.clone());
        members
    }

    #[cfg(test)]
    pub(crate) fn resolve_candidates(
        &self,
        logical_path: &str,
        api_type: ApiType,
    ) -> Result<CandidateSet, ModelRegistryError> {
        self.resolve_available_candidates(logical_path, api_type, &mut |_, _| true)
    }

    pub(crate) fn reachable_models(
        &self,
        logical_path: &str,
        api_type: ApiType,
    ) -> Result<Vec<RegisteredModel>, ModelRegistryError> {
        validate_logical_path(logical_path)?;
        ensure_path_api_namespace(logical_path, api_type)?;
        let mut models = Vec::<RegisteredModel>::new();
        let mut current = logical_path.to_owned();
        let mut visited = BTreeSet::new();
        while visited.insert(current.clone()) && visited.len() <= self.fallback_depth_limit + 1 {
            let mut accept = |_: &str, _: &RegistryCandidate| true;
            let mut context = ExpandContext {
                resolved_path: &current,
                accept: &mut accept,
                explore_all: true,
                admissions: Vec::new(),
                expansions: Vec::new(),
            };
            let reached = self.expand_path(
                &current,
                api_type,
                &mut BTreeSet::new(),
                &mut Vec::new(),
                empty_candidate_path(),
                &mut context,
            )?;
            for candidate in reached {
                if !models
                    .iter()
                    .any(|model| model.exact_model == candidate.model.exact_model)
                {
                    models.push(candidate.model);
                }
            }
            match self.fallback_target(&current, api_type)? {
                FallbackTarget::None => break,
                FallbackTarget::Logical(next) => current = next,
                FallbackTarget::Exact(exact) => {
                    for candidate in self.exact_fallback_candidate(&exact, api_type) {
                        if !models
                            .iter()
                            .any(|model| model.exact_model == candidate.model.exact_model)
                        {
                            models.push(candidate.model);
                        }
                    }
                    break;
                }
            }
        }
        Ok(models)
    }

    pub(crate) fn resolve_available_candidates(
        &self,
        logical_path: &str,
        api_type: ApiType,
        accept: &mut dyn FnMut(&str, &RegistryCandidate) -> bool,
    ) -> Result<CandidateSet, ModelRegistryError> {
        validate_logical_path(logical_path)?;
        ensure_path_api_namespace(logical_path, api_type)?;
        let requested = logical_path.to_owned();
        let mut current = requested.clone();
        let mut fallback_chain = Vec::new();
        let mut all_admissions = Vec::new();
        let mut all_expansions = Vec::new();
        let mut visited = BTreeSet::new();
        let mut requirements = Vec::new();
        loop {
            if !visited.insert(current.clone()) {
                return Err(ModelRegistryError::FallbackLoop(current));
            }
            if fallback_chain.len() > self.fallback_depth_limit {
                return Err(ModelRegistryError::FallbackDepthExceeded(
                    self.fallback_depth_limit,
                ));
            }
            let mut context = ExpandContext {
                resolved_path: &current,
                accept: &mut *accept,
                explore_all: false,
                admissions: Vec::new(),
                expansions: Vec::new(),
            };
            let candidates = self.expand_path(
                &current,
                api_type,
                &mut BTreeSet::new(),
                &mut requirements.clone(),
                empty_candidate_path(),
                &mut context,
            )?;
            let ExpandContext {
                mut admissions,
                expansions,
                ..
            } = context;
            normalize_admissions(&mut admissions);
            all_admissions.extend(admissions);
            normalize_admissions(&mut all_admissions);
            all_expansions.extend(expansions);
            requirements.push(self.logical_requirement(&current));
            if !candidates.is_empty() {
                let disable_line = self.disable_line(&requested);
                let default_options = self.default_options(&requested);
                let scheduler_profile = self.scheduler_profile(&requested);
                return Ok(CandidateSet {
                    requested_logical_path: requested,
                    resolved_logical_path: current,
                    candidates,
                    admissions: all_admissions,
                    expansions: all_expansions,
                    fallback_chain,
                    disable_line,
                    default_options,
                    scheduler_profile,
                });
            }
            match self.fallback_target(&current, api_type)? {
                FallbackTarget::None => {
                    let disable_line = self.disable_line(&requested);
                    let default_options = self.default_options(&requested);
                    let scheduler_profile = self.scheduler_profile(&requested);
                    return Ok(CandidateSet {
                        requested_logical_path: requested,
                        resolved_logical_path: current,
                        candidates,
                        admissions: all_admissions,
                        expansions: all_expansions,
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
                    let mut candidates = self.exact_fallback_candidate(&exact, api_type);
                    candidates.retain(|candidate| {
                        requirements.iter().all(|requirement| {
                            missing_requirements(requirement, &candidate.model).is_empty()
                        }) && accept(&exact, candidate)
                    });
                    let disable_line = self.disable_line(&requested);
                    fallback_chain.push(FallbackStep {
                        from: current,
                        to: exact.clone(),
                    });
                    return Ok(CandidateSet {
                        requested_logical_path: requested,
                        resolved_logical_path: exact,
                        candidates,
                        admissions: all_admissions,
                        expansions: all_expansions,
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

    fn register_specs(&mut self, catalog: &CatalogSnapshot) -> Result<(), ModelRegistryError> {
        for (driver, id, model) in catalog.llm_models() {
            if self.logical_nodes.contains_key(&model.family) {
                return Err(ModelRegistryError::InvalidLogicalTree(format!(
                    "{driver} model {id}: family {} conflicts with a task",
                    model.family
                )));
            }
        }
        for driver in catalog.model_drivers() {
            for spec in &driver.specs {
                let path = format!("llm.{}", spec.id);
                if self.logical_nodes.contains_key(&path) {
                    return Err(ModelRegistryError::InvalidLogicalTree(format!(
                        "{} spec {} conflicts with a task",
                        driver.model_driver_id, spec.id
                    )));
                }
                self.specs.insert(
                    path.clone(),
                    (driver.model_driver_id.clone(), spec.direct_only),
                );
                self.logical_nodes.entry(path).or_default().fallback = disabled_fallback();
            }
        }
        Ok(())
    }

    fn materialize_families(&mut self, catalog: &CatalogSnapshot) {
        let mut families: BTreeMap<String, (LlmModel, Vec<String>)> = BTreeMap::new();
        for model in self
            .models
            .values()
            .filter(|model| model.api_types.contains(&ApiType::Llm))
        {
            let Some(family) = catalog.llm_model(
                &model.identity.model_driver_id,
                &model.identity.origin_model_id,
            ) else {
                continue;
            };
            let (_, members) = families
                .entry(family.family.clone())
                .or_insert_with(|| (family.clone(), Vec::new()));
            if family
                .semantics
                .supported_efforts
                .iter()
                .any(|effort| effort.variant().as_deref() == model.exact_model.variant())
            {
                members.push(model.exact_model.to_string());
            }
        }
        for (path, (family, members)) in families {
            let node = self.logical_nodes.entry(path).or_default();
            node.fallback = disabled_fallback();
            for exact in members {
                node.family_members.insert(exact.clone());
                node.items.push(EffectiveItem::new(
                    exact.clone(),
                    exact,
                    1.0,
                    LogicalItemSource::DriverMetadataMount,
                ));
            }
            let spec = format!("llm.{}", family.semantics.spec);
            let item = EffectiveItem::new(
                family.family.clone(),
                format!("{}:{}", family.family, family.semantics.effort.as_str()),
                family.semantics.weight,
                LogicalItemSource::DriverMetadataMount,
            );
            node.family = Some(family);
            self.logical_nodes
                .get_mut(&spec)
                .expect("catalog validated spec")
                .items
                .push(item);
        }
    }

    fn validate_llm_tree(&self) -> Result<(), ModelRegistryError> {
        let invalid = |reason: String| ModelRegistryError::InvalidLogicalTree(reason);
        let mut referenced = BTreeSet::new();
        for (path, node) in &self.logical_nodes {
            let llm = path_namespace(path) == "llm";
            if llm
                && node
                    .definition
                    .as_ref()
                    .is_some_and(|d| d.mount_mode != MountMode::Manual)
            {
                return Err(invalid(format!(
                    "{path}: LLM directories require manual specification admission"
                )));
            }
            if llm
                && node
                    .fallback
                    .as_ref()
                    .is_some_and(|r| r.mode == AiccFallbackMode::Parent)
            {
                return Err(invalid(format!("{path}: LLM parent fallback is forbidden")));
            }
            if path == "llm"
                && (!node.items.is_empty()
                    || node.fallback.as_ref().is_some_and(|r| r.target.is_some()))
            {
                return Err(invalid("llm is a namespace".into()));
            }
            if self.family_names.contains(path) && node.family.is_none() && !node.items.is_empty() {
                return Err(invalid(format!("{path}: family requires inventory")));
            }
            for item in &node.items {
                let target = &item.target;
                let target_path = target.split(':').next().unwrap_or(target);
                if (llm && node.family.is_none() && !self.specs.contains_key(path))
                    || (!llm && path_namespace(target_path) == "llm")
                {
                    if !self.specs.contains_key(target) {
                        return Err(invalid(format!(
                            "{path} must reference a declared specification, got {target}"
                        )));
                    }
                }
                if node.family.is_some()
                    && (item.name != *target || !node.family_members.contains(target))
                {
                    return Err(invalid(format!(
                        "{path}: family members are inventory facts"
                    )));
                }
                if self.specs.contains_key(path) {
                    let Some(family) = self
                        .logical_nodes
                        .get(target_path)
                        .and_then(|node| node.family.as_ref())
                    else {
                        return Err(invalid(format!("{path}: unknown family {target}")));
                    };
                    if path != &format!("llm.{}", family.semantics.spec)
                        || target
                            != &format!("{}:{}", family.family, family.semantics.effort.as_str())
                    {
                        return Err(invalid(format!("{path}: specification membership and effort come from metadata: {target}")));
                    }
                }
                if let Some((driver, direct_only)) = self.specs.get(target_path) {
                    if *direct_only {
                        return Err(invalid(format!(
                            "{path} references direct_only {driver} spec {target_path}"
                        )));
                    }
                    referenced.insert(target_path.to_owned());
                }
                if !target.contains('@')
                    && (llm || path_namespace(target_path) == "llm")
                    && !self.logical_nodes.contains_key(target_path)
                {
                    return Err(invalid(format!("{path}: dangling reference {target}")));
                }
            }
            if let Some(target) = node.fallback.as_ref().and_then(|rule| rule.target.as_ref()) {
                if let Some((driver, direct)) = self.specs.get(target) {
                    if *direct {
                        return Err(invalid(format!(
                            "{path}: fallback references direct_only {driver} spec {target}"
                        )));
                    }
                    referenced.insert(target.clone());
                }
                if let Some(family) = self
                    .logical_nodes
                    .get(target.split(':').next().unwrap_or(target))
                    .and_then(|node| node.family.as_ref())
                {
                    if self.specs[&format!("llm.{}", family.semantics.spec)].1 {
                        return Err(invalid(format!(
                            "{path}: fallback bypasses direct_only via {target}"
                        )));
                    }
                }
                if let Some(model) = self.models.get(target) {
                    for family in self
                        .logical_nodes
                        .values()
                        .filter_map(|node| node.family.as_ref())
                    {
                        if self.specs[&format!("llm.{}", family.semantics.spec)].1
                            && family.model_driver_id == model.identity.model_driver_id
                            && family.origin_model_id == model.identity.origin_model_id
                        {
                            return Err(invalid(format!(
                                "{path}: fallback bypasses direct_only via {target}"
                            )));
                        }
                    }
                }
                if llm
                    && !target.contains('@')
                    && !self
                        .logical_nodes
                        .contains_key(target.split(':').next().unwrap_or(target))
                {
                    return Err(invalid(format!("{path}: dangling fallback {target}")));
                }
            }
        }
        for (path, (driver, direct_only)) in &self.specs {
            if !direct_only && !referenced.contains(path) {
                return Err(invalid(format!(
                    "{driver} spec {path} is neither referenced nor direct_only"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn logical_requirement(&self, path: &str) -> ModelRequirement {
        self.logical_nodes
            .get(path)
            .and_then(|node| node.definition.as_ref())
            .map(|definition| definition.min_line.clone())
            .unwrap_or_default()
    }

    fn experimental(&self, path: &CandidatePath) -> bool {
        path.logical_paths.iter().any(|path| {
            self.logical_nodes
                .get(path.split(':').next().unwrap_or(path))
                .and_then(|node| node.family.as_ref())
                .is_some_and(|family| family.semantics.stability == ModelStability::Experimental)
        })
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
                if catalog
                    .resolve_model(&model.model_driver_id, &model.origin_model_id)
                    .is_err()
                {
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
                    if let Some(llm) =
                        catalog.llm_model(&model.model_driver_id, &model.origin_model_id)
                    {
                        if !llm
                            .semantics
                            .supported_efforts
                            .iter()
                            .any(|effort| effort.variant().as_deref() == Some(&variant.name))
                        {
                            return Err(ModelRegistryError::InvalidLogicalTree(format!(
                                "illegal preset {} for {}/{}",
                                variant.name, model.model_driver_id, model.origin_model_id
                            )));
                        }
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
        if model.api_types.contains(&ApiType::Llm) {
            logical_mounts.clear();
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
            canonical_fields: model.canonical_fields.clone(),
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
                definition.api_type != ApiType::Llm
                    && matches!(definition.mount_mode, MountMode::Auto | MountMode::Hybrid)
            })
            .collect::<Vec<_>>();
        let models = self.models.values().cloned().collect::<Vec<_>>();
        for definition in definitions {
            for model in &models {
                if model.api_types.contains(&ApiType::Llm)
                    || !model.api_types.contains(&definition.api_type)
                {
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
        let items = &mut self.logical_nodes.entry(path.to_owned()).or_default().items;
        if !items.iter().any(|item| item.name == exact_model) {
            items.push(EffectiveItem::new(exact_model, exact_model, 1.0, source));
        }
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
            if source == LogicalItemSource::BuiltinDefinition {
                if node
                    .definition
                    .as_ref()
                    .is_some_and(|definition| definition.mount_mode == MountMode::Manual)
                {
                    node.items = effective_items(items, source);
                } else {
                    merge_items(&mut node.items, effective_items(items, source));
                }
            } else {
                node.items = effective_items(items, source);
                if overlay.fallback.is_none() {
                    node.fallback = disabled_fallback();
                }
            }
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
                merge_items(&mut node.items, effective_items(&overlay.items, source));
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

    fn expand_path(
        &self,
        logical_path: &str,
        api_type: ApiType,
        stack: &mut BTreeSet<String>,
        requirements: &mut Vec<ModelRequirement>,
        path: CandidatePath,
        context: &mut ExpandContext<'_>,
    ) -> Result<Vec<RegistryCandidate>, ModelRegistryError> {
        if !stack.insert(logical_path.to_owned()) {
            return Err(ModelRegistryError::LogicalTreeLoop(logical_path.to_owned()));
        }
        let (base_path, selector) = logical_path
            .split_once(':')
            .map_or((logical_path, None), |(base, effort)| (base, Some(effort)));
        let Some(node) = self.logical_nodes.get(base_path) else {
            stack.remove(logical_path);
            return Ok(Vec::new());
        };
        let effort = if let Some(family) = &node.family {
            let selected = selector.unwrap_or(family.semantics.default_effort.as_str());
            let Some(effort) = family
                .semantics
                .supported_efforts
                .iter()
                .find(|effort| effort.as_str() == selected)
            else {
                stack.remove(logical_path);
                return Ok(Vec::new());
            };
            Some(*effort)
        } else if selector.is_some() {
            return Err(ModelRegistryError::InvalidLogicalPath(logical_path.into()));
        } else {
            None
        };
        context.admissions.extend(node.admissions.values().cloned());
        if let Some(definition) = &node.definition {
            if definition.api_type != api_type {
                stack.remove(logical_path);
                return Ok(Vec::new());
            }
            requirements.push(definition.min_line.clone());
        }
        let items = node
            .items
            .iter()
            .filter(|item| {
                effort.is_none_or(|effort| {
                    self.models.get(&item.target).is_some_and(|model| {
                        model.exact_model.variant() == effort.variant().as_deref()
                    })
                })
            })
            .filter(|item| item.weight > 0.0)
            .collect::<Vec<_>>();
        let mut weights = items.iter().map(|item| item.weight).collect::<Vec<_>>();
        weights.sort_by(|left, right| right.total_cmp(left));
        weights.dedup();
        let mut pool = Vec::new();
        let mut selected = None;
        for weight in weights {
            for item in items.iter().filter(|item| item.weight == weight) {
                let mut next_path = path.clone();
                next_path.logical_paths.push(logical_path.to_owned());
                next_path.item_names.push(item.name.clone());
                next_path.weights.push(item.weight);
                next_path.sources.push(item.source);
                let reached = if item.target.contains('@') {
                    self.leaf_candidate(
                        logical_path,
                        &item.target,
                        api_type,
                        requirements,
                        next_path,
                        context,
                    )
                } else {
                    ensure_same_namespace(logical_path, &item.target)?;
                    self.expand_path(
                        &item.target,
                        api_type,
                        stack,
                        requirements,
                        next_path,
                        context,
                    )?
                };
                merge_candidates(&mut pool, reached);
            }
            if !pool.is_empty() && !context.explore_all {
                selected = Some(weight);
                break;
            }
        }
        if !context.explore_all
            && !context
                .expansions
                .iter()
                .any(|step| step.logical_path == logical_path)
        {
            context.expansions.push(ExpansionStep {
                logical_path: logical_path.to_owned(),
                max_weight: selected,
                items: items
                    .iter()
                    .map(|item| ExpansionItem {
                        name: item.name.clone(),
                        target: item.target.clone(),
                        weight: item.weight,
                        weight_source: item.weight_source,
                        state: match selected {
                            Some(weight) if item.weight == weight => ExpansionState::Expanded,
                            Some(weight) if item.weight < weight => ExpansionState::NotExpanded,
                            _ => ExpansionState::Unavailable,
                        },
                    })
                    .collect(),
            });
        }
        if node.definition.is_some() {
            requirements.pop();
        }
        stack.remove(logical_path);
        Ok(pool)
    }

    fn leaf_candidate(
        &self,
        logical_path: &str,
        exact_model: &str,
        api_type: ApiType,
        requirements: &[ModelRequirement],
        path: CandidatePath,
        context: &mut ExpandContext<'_>,
    ) -> Vec<RegistryCandidate> {
        let Some(model) = self.models.get(exact_model) else {
            return Vec::new();
        };
        if !model.api_types.contains(&api_type) {
            return Vec::new();
        }
        let missing = requirements
            .iter()
            .flat_map(|requirement| missing_requirements(requirement, model))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        context.admissions.push(AdmissionRecord {
            logical_path: logical_path.to_owned(),
            exact_model: exact_model.to_owned(),
            admitted: missing.is_empty(),
            missing_requirements: missing.clone(),
        });
        if !missing.is_empty() {
            return Vec::new();
        }
        let exact_model_weight = path
            .logical_paths
            .iter()
            .find_map(|path| {
                self.logical_nodes
                    .get(path)
                    .and_then(|node| node.exact_model_weights.get(exact_model))
            })
            .or_else(|| self.global_exact_model_weights.get(exact_model))
            .copied()
            .unwrap_or(1.0);
        if exact_model_weight == 0.0 {
            return Vec::new();
        }
        let candidate = RegistryCandidate {
            model: model.clone(),
            experimental: self.experimental(&path),
            paths: vec![path],
            exact_model_weight,
            provider_weight: self
                .provider_weights
                .get(&model.identity.provider_instance_name)
                .copied()
                .unwrap_or(1.0),
        };
        if (context.accept)(context.resolved_path, &candidate) {
            vec![candidate]
        } else {
            Vec::new()
        }
    }

    fn exact_fallback_candidate(&self, exact: &str, api_type: ApiType) -> Vec<RegistryCandidate> {
        self.models
            .get(exact)
            .filter(|model| model.api_types.contains(&api_type))
            .map(|model| RegistryCandidate {
                model: model.clone(),
                paths: Vec::new(),
                experimental: false,
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
            .get(path.split(':').next().unwrap_or(path))
            .and_then(|node| node.fallback.as_ref());
        let mode =
            fallback
                .map(|fallback| &fallback.mode)
                .unwrap_or(if path_namespace(path) == "llm" {
                    &AiccFallbackMode::Strict
                } else {
                    &AiccFallbackMode::Parent
                });
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

    pub(crate) fn disable_line(&self, path: &str) -> ModelDisable {
        self.logical_nodes
            .get(path.split(':').next().unwrap_or(path))
            .map(|node| node.disable_line.clone())
            .unwrap_or_default()
    }

    fn default_options(&self, path: &str) -> BTreeMap<String, Value> {
        self.logical_nodes
            .get(path.split(':').next().unwrap_or(path))
            .and_then(|node| node.definition.as_ref())
            .map(|definition| definition.default_options.clone())
            .unwrap_or_default()
    }

    fn scheduler_profile(&self, path: &str) -> AiccSchedulerProfile {
        self.logical_nodes
            .get(path.split(':').next().unwrap_or(path))
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
            for item in &node.items {
                let target = item.target.split(':').next().unwrap_or(&item.target);
                if !item.target.contains('@') && self.logical_nodes.contains_key(target) {
                    ensure_same_namespace(path, target)?;
                    self.visit_item_graph(target, visiting, complete)?;
                }
            }
            if let Some(target) = node
                .fallback
                .as_ref()
                .and_then(|rule| rule.target.as_ref())
                .filter(|target| !target.contains('@'))
            {
                let target = target.split(':').next().unwrap_or(target);
                self.visit_item_graph(target, visiting, complete)?;
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
            provider_model_id: model.exact_model.variant().map_or_else(
                || model.identity.provider_model_id.clone(),
                |variant| format!("{}:{variant}", model.identity.provider_model_id),
            ),
            variant: model.exact_model.variant.clone(),
            api_types: model
                .api_types
                .iter()
                .map(|api_type| api_type_name(*api_type).to_owned())
                .collect(),
            logical_mounts: model.logical_mounts.clone(),
            capabilities: model.capabilities.clone(),
            canonical_fields: model.canonical_fields.clone(),
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
        ApiType::Decision => "decision",
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
    if path_namespace(from) == path_namespace(to)
        || (path_namespace(from) != "llm" && path_namespace(to) == "llm")
    {
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
        || path.contains('{')
        || path.contains('}')
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

fn effective_items(items: &[LogicalItem], source: LogicalItemSource) -> Vec<EffectiveItem> {
    items
        .iter()
        .map(|item| EffectiveItem::new(&item.name, &item.target, item.weight, source))
        .collect()
}

fn target_base(target: &str) -> &str {
    if target.contains('@') {
        target
    } else {
        target.split(':').next().unwrap_or(target)
    }
}

fn empty_candidate_path() -> CandidatePath {
    CandidatePath {
        logical_paths: Vec::new(),
        item_names: Vec::new(),
        weights: Vec::new(),
        sources: Vec::new(),
    }
}

fn merge_candidates(pool: &mut Vec<RegistryCandidate>, additions: Vec<RegistryCandidate>) {
    for addition in additions {
        if let Some(candidate) = pool
            .iter_mut()
            .find(|candidate| candidate.model.exact_model == addition.model.exact_model)
        {
            candidate.experimental |= addition.experimental;
            candidate.paths.extend(addition.paths);
        } else {
            pool.push(addition);
        }
    }
}

fn merge_items(items: &mut Vec<EffectiveItem>, additions: Vec<EffectiveItem>) {
    for addition in additions {
        if let Some(item) = items.iter_mut().find(|item| item.name == addition.name) {
            *item = addition;
        } else {
            items.push(addition);
        }
    }
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

fn validate_items(path: &str, items: &[LogicalItem]) -> Result<(), ModelRegistryError> {
    let mut names = BTreeSet::new();
    for item in items {
        if item.name.is_empty() {
            return Err(ModelRegistryError::InvalidItemName(path.to_owned()));
        }
        if !names.insert(item.name.as_str()) {
            return Err(ModelRegistryError::DuplicateItemName {
                path: path.to_owned(),
                item: item.name.clone(),
            });
        }
        validate_weight(item.weight, format!("{path}.items.{}.weight", item.name))?;
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
    items: &mut Vec<EffectiveItem>,
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
        if let Some(item) = items.iter_mut().find(|item| &item.name == name) {
            if let Some(target) = &patch.target {
                item.target = target.clone();
                item.source = source;
            }
            if let Some(weight) = patch.weight {
                item.weight = weight;
                item.weight_source = source;
            }
        } else if let Some(target) = &patch.target {
            items.push(EffectiveItem::new(
                name,
                target,
                patch.weight.unwrap_or(1.0),
                source,
            ));
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
    if let Some(decision) = &requirement.decision {
        for feature in decision.features() {
            if model.capabilities.get(&feature).and_then(Value::as_bool) != Some(true) {
                missing.push(feature);
            }
        }
        for (limit, required) in decision.limits() {
            if model
                .capabilities
                .get(limit)
                .and_then(Value::as_u64)
                .unwrap_or_default()
                < required
            {
                missing.push(format!("{limit}:{required}"));
            }
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
    for (field, required) in &requirement.canonical_fields {
        if !crate::canonical::resolve_canonical_field(model.canonical_fields.get(field), required)
            .satisfies(required)
        {
            missing.push(format!("canonical_field:{field}"));
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
            Self::InvalidLogicalTree(reason) => write!(formatter, "invalid logical tree: {reason}"),
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
            Self::DuplicateItemName { path, item } => {
                write!(formatter, "`{path}` has duplicate item `{item}`")
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
    use buckyos_api::AiccSessionLogicalProfile;
    use serde_json::json;

    fn catalog() -> CatalogSnapshot {
        let driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 2,
            "schema_revision": 0,
            "model_driver_id": "openai",
            "revision_seq": 1,
            "models": [],
            "patterns": [{
                "match": {"origin_model_id": ["gpt-5.2", "a", "b", "capable", "basic", "gpt", "mini", "unrecognizable-model", "computer", "bad"]},
                "api_types": ["image.txt2img"],
                "capabilities": {"streaming": true}
            }],
            "defaults": {},
            "specs": []
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
            api_types: vec![ApiType::ImageTextToImage],
            logical_mounts: vec!["image.gpt".to_owned()],
            variants: Vec::new(),
            capabilities: BTreeMap::from([
                ("streaming".to_owned(), json!(true)),
                ("tool_call".to_owned(), json!(tool_call)),
                ("max_context_tokens".to_owned(), json!(128_000)),
            ]),
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::from([(
                "image.txt2img".to_owned(),
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
            api_type: ApiType::ImageTextToImage,
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
                    .map(|(name, target, weight)| LogicalItem::new(*name, *target, *weight))
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
            logical_mounts: vec!["image.reason".to_owned()],
        });
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![model])],
            vec![
                definition("image.gpt", false, MountMode::Manual),
                definition("image.reason", true, MountMode::Manual),
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
            ModelView::from(variant).provider_model_id,
            "gpt-5.2:reasoning-high"
        );
        assert_eq!(
            registry
                .resolve_candidates("image.reason", ApiType::ImageTextToImage)
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
            vec![definition("image.plan", true, MountMode::Auto)],
            RegistryLayers::default(),
        )
        .unwrap();
        let result = registry
            .resolve_candidates("image.plan", ApiType::ImageTextToImage)
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
                    "image.plan".to_owned(),
                    item_node(&[("a", "image.family_a", 2.0), ("b", "image.family_b", 2.0)]),
                ),
                (
                    "image.family_a".to_owned(),
                    item_node(&[("model", "gpt@primary", 1.0)]),
                ),
                (
                    "image.family_b".to_owned(),
                    item_node(&[("model", "gpt@primary", 1.0), ("mini", "mini@primary", 1.0)]),
                ),
            ]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory(
                "primary",
                vec![inventory_model("gpt", true), inventory_model("mini", true)],
            )],
            vec![definition("image.plan", false, MountMode::Manual)],
            RegistryLayers {
                system: Some(&overlay),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("image.plan", ApiType::ImageTextToImage)
            .unwrap();
        let order: Vec<_> = result
            .candidates
            .iter()
            .map(|candidate| candidate.model.exact_model.as_str())
            .collect();
        assert_eq!(order, ["gpt@primary", "mini@primary"]);
        assert_eq!(result.candidates[0].paths.len(), 2);
        assert_eq!(result.candidates[0].paths[0].weights, vec![2.0, 1.0]);
        assert_eq!(result.candidates[0].paths[1].item_names, vec!["b", "model"]);
    }

    #[test]
    fn each_directory_expands_only_its_highest_available_weight_group() {
        let overlay = AiccRouteOverlay {
            logical_tree: BTreeMap::from([
                (
                    "image.plan".to_owned(),
                    item_node(&[
                        ("low", "image.low", 1.8),
                        ("left", "image.left", 2.0),
                        ("right", "image.right", 2.0),
                    ]),
                ),
                (
                    "image.left".to_owned(),
                    item_node(&[("old", "a@primary", 55.0), ("new", "b@primary", 56.0)]),
                ),
                (
                    "image.right".to_owned(),
                    item_node(&[
                        ("old", "capable@primary", 48.0),
                        ("new", "gpt@primary", 55.0),
                    ]),
                ),
                (
                    "image.low".to_owned(),
                    item_node(&[("model", "mini@primary", 1.0)]),
                ),
            ]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory(
                "primary",
                ["a", "b", "capable", "gpt", "mini"]
                    .into_iter()
                    .map(|id| inventory_model(id, true))
                    .collect(),
            )],
            vec![definition("image.plan", false, MountMode::Manual)],
            RegistryLayers {
                system: Some(&overlay),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let names = |set: &CandidateSet| {
            set.candidates
                .iter()
                .map(|candidate| candidate.model.exact_model.to_string())
                .collect::<Vec<_>>()
        };
        let result = registry
            .resolve_candidates("image.plan", ApiType::ImageTextToImage)
            .unwrap();
        assert_eq!(names(&result), ["b@primary", "gpt@primary"]);
        let plan = result
            .expansions
            .iter()
            .find(|step| step.logical_path == "image.plan")
            .unwrap();
        assert_eq!(plan.max_weight, Some(2.0));
        assert_eq!(
            plan.items
                .iter()
                .map(|item| (item.name.as_str(), item.state))
                .collect::<Vec<_>>(),
            [
                ("low", ExpansionState::NotExpanded),
                ("left", ExpansionState::Expanded),
                ("right", ExpansionState::Expanded),
            ]
        );
        assert!(!result
            .expansions
            .iter()
            .any(|step| step.logical_path == "image.low"));

        let result = registry
            .resolve_available_candidates(
                "image.plan",
                ApiType::ImageTextToImage,
                &mut |_, candidate| candidate.model.exact_model.as_str() != "b@primary",
            )
            .unwrap();
        assert_eq!(names(&result), ["a@primary", "gpt@primary"]);

        let result = registry
            .resolve_available_candidates(
                "image.plan",
                ApiType::ImageTextToImage,
                &mut |_, candidate| candidate.model.exact_model.as_str() == "mini@primary",
            )
            .unwrap();
        assert_eq!(names(&result), ["mini@primary"]);
        let plan = result
            .expansions
            .iter()
            .find(|step| step.logical_path == "image.plan")
            .unwrap();
        assert_eq!(plan.max_weight, Some(1.8));
        assert_eq!(plan.items[1].state, ExpansionState::Unavailable);

        assert!(registry
            .resolve_available_candidates("image.plan", ApiType::ImageTextToImage, &mut |_, _| {
                false
            },)
            .unwrap()
            .candidates
            .is_empty());
        let reachable = registry
            .reachable_models("image.plan", ApiType::ImageTextToImage)
            .unwrap();
        assert_eq!(reachable.len(), 5);
    }

    #[test]
    fn weight_only_overrides_keep_position_and_record_sources() {
        let factory = layer(
            "image.chat",
            &[
                ("second", "gpt@primary", 1.0),
                ("first", "mini@primary", 1.0),
            ],
        );
        let user = AiccRouteOverlay {
            logical_tree: BTreeMap::from([(
                "image.chat".to_owned(),
                AiccLogicalNodeOverlay {
                    item_overrides: Some(BTreeMap::from([
                        (
                            "first".to_owned(),
                            ModelItemPatch {
                                weight: Some(3.0),
                                ..ModelItemPatch::default()
                            },
                        ),
                        (
                            "missing".to_owned(),
                            ModelItemPatch {
                                weight: Some(9.0),
                                ..ModelItemPatch::default()
                            },
                        ),
                    ])),
                    ..AiccLogicalNodeOverlay::default()
                },
            )]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory(
                "primary",
                vec![inventory_model("gpt", true), inventory_model("mini", true)],
            )],
            vec![definition("image.chat", false, MountMode::Manual)],
            RegistryLayers {
                factory: Some(&factory),
                user: Some(&user),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let view = registry
            .logical_model_views()
            .into_iter()
            .find(|view| view.path == "image.chat")
            .unwrap();
        assert_eq!(
            view.items
                .iter()
                .map(|item| (
                    item.name.as_str(),
                    item.weight,
                    item.default_weight,
                    item.source,
                    item.weight_source
                ))
                .collect::<Vec<_>>(),
            [
                (
                    "second",
                    1.0,
                    1.0,
                    LogicalItemSource::BuiltinDefinition,
                    LogicalItemSource::BuiltinDefinition
                ),
                (
                    "first",
                    3.0,
                    1.0,
                    LogicalItemSource::BuiltinDefinition,
                    LogicalItemSource::UserOverlay
                ),
            ]
        );
        let result = registry
            .resolve_candidates("image.chat", ApiType::ImageTextToImage)
            .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].model.exact_model.as_str(),
            "mini@primary"
        );
    }

    #[test]
    fn duplicate_item_names_are_rejected() {
        let duplicate = layer(
            "image.chat",
            &[("same", "gpt@primary", 1.0), ("same", "mini@primary", 1.0)],
        );
        assert!(matches!(
            ModelRegistry::build(
                &catalog(),
                &[],
                Vec::new(),
                RegistryLayers {
                    system: Some(&duplicate),
                    ..RegistryLayers::default()
                }
            ),
            Err(ModelRegistryError::DuplicateItemName { .. })
        ));
    }

    #[test]
    fn parent_exact_model_weight_filters_candidates_reached_through_links() {
        let overlay = AiccRouteOverlay {
            logical_tree: BTreeMap::from([
                (
                    "image.plan".to_owned(),
                    AiccLogicalNodeOverlay {
                        items: Some(vec![LogicalItem::new("family", "image.family", 1.0)]),
                        exact_model_weights: BTreeMap::from([("gpt@primary".to_owned(), 0.0)]),
                        ..AiccLogicalNodeOverlay::default()
                    },
                ),
                (
                    "image.family".to_owned(),
                    item_node(&[("model", "gpt@primary", 1.0)]),
                ),
            ]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", true)])],
            vec![definition("image.plan", false, MountMode::Manual)],
            RegistryLayers {
                session: Some(&overlay),
                ..RegistryLayers::default()
            },
        )
        .unwrap();

        assert!(registry
            .resolve_candidates("image.plan", ApiType::ImageTextToImage)
            .unwrap()
            .candidates
            .is_empty());
    }

    #[test]
    fn direct_items_replacement_disables_fallback_by_default() {
        let factory = layer("image", &[("default", "gpt@primary", 1.0)]);
        let replacement = AiccRouteOverlay {
            logical_tree: BTreeMap::from([(
                "image.manual".to_owned(),
                AiccLogicalNodeOverlay {
                    items: Some(Vec::new()),
                    ..AiccLogicalNodeOverlay::default()
                },
            )]),
            ..AiccRouteOverlay::default()
        };
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", true)])],
            Vec::new(),
            RegistryLayers {
                factory: Some(&factory),
                session: Some(&replacement),
                ..RegistryLayers::default()
            },
        )
        .unwrap();

        let result = registry
            .resolve_candidates("image.manual", ApiType::ImageTextToImage)
            .unwrap();
        assert!(result.candidates.is_empty());
        assert!(result.fallback_chain.is_empty());
    }

    #[test]
    fn factory_user_and_session_overlays_compose_in_order() {
        let factory = layer("image.chat", &[("primary", "gpt@primary", 1.0)]);
        let user = AiccRouteOverlay {
            logical_profile: Some(AiccSessionLogicalProfile {
                overlays: vec![AiccLogicalTreeOverlay {
                    path: "image.chat".to_owned(),
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
                    path: "image.chat".to_owned(),
                    merge_mode: OverlayMergeMode::Replace,
                    items: vec![LogicalItem::new("only", "mini@backup", 4.0)],
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
            vec![definition("image.chat", false, MountMode::Manual)],
            RegistryLayers {
                factory: Some(&factory),
                user: Some(&user),
                session: Some(&session),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("image.chat", ApiType::ImageTextToImage)
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
            .find(|view| view.path == "image.chat")
            .unwrap();
        assert_eq!(view.fallback.unwrap().mode, AiccFallbackMode::Disabled);
    }

    #[test]
    fn invalid_overlay_forms_and_weights_are_rejected() {
        let conflict = AiccRouteOverlay {
            logical_tree: BTreeMap::from([(
                "image.chat".to_owned(),
                AiccLogicalNodeOverlay {
                    items: Some(Vec::new()),
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

        let invalid = layer("image.chat", &[("bad", "gpt@primary", -1.0)]);
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
                ("image.a".to_owned(), item_node(&[("b", "image.b", 1.0)])),
                ("image.b".to_owned(), item_node(&[("a", "image.a", 1.0)])),
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

        let cross = layer("image.chat", &[("bad", "audio.tts", 1.0)]);
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
        let factory = layer("image", &[("model", "gpt@primary", 1.0)]);
        let registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", false)])],
            vec![LogicalModelDefinition {
                fallback: Some(AiccFallbackRule {
                    mode: AiccFallbackMode::Parent,
                    target: None,
                }),
                ..definition("image.code", true, MountMode::Auto)
            }],
            RegistryLayers {
                factory: Some(&factory),
                ..RegistryLayers::default()
            },
        )
        .unwrap();
        let result = registry
            .resolve_candidates("image.code", ApiType::ImageTextToImage)
            .unwrap();
        assert_eq!(result.resolved_logical_path, "image");
        assert_eq!(result.fallback_chain.len(), 1);
        assert!(result.candidates.is_empty());
        assert!(result.admissions.iter().any(|record| {
            record.logical_path == "image.code"
                && record.exact_model == "gpt@primary"
                && !record.admitted
        }));

        let exact_definition = LogicalModelDefinition {
            fallback: Some(AiccFallbackRule {
                mode: AiccFallbackMode::TargetExact,
                target: Some("gpt@primary".to_owned()),
            }),
            ..definition("image.strict", false, MountMode::Manual)
        };
        let exact_registry = ModelRegistry::build(
            &catalog(),
            &[inventory("primary", vec![inventory_model("gpt", true)])],
            vec![exact_definition],
            RegistryLayers::default(),
        )
        .unwrap();
        let exact = exact_registry
            .resolve_candidates("image.strict", ApiType::ImageTextToImage)
            .unwrap();
        assert_eq!(exact.resolved_logical_path, "gpt@primary");
        assert_eq!(exact.candidates.len(), 1);
    }

    #[test]
    fn fallback_loops_and_excess_depth_are_rejected() {
        let mut loop_overlay = AiccRouteOverlay::default();
        for (path, target) in [("image.a", "image.b"), ("image.b", "image.a")] {
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
                format!("image.d{index}"),
                AiccLogicalNodeOverlay {
                    fallback: Some(AiccFallbackRule {
                        mode: AiccFallbackMode::TargetLogical,
                        target: Some(format!("image.d{}", index + 1)),
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
            vec![definition("image.chat", true, MountMode::Auto)],
            RegistryLayers::default(),
        )
        .unwrap();
        let model = registry
            .exact_model("unrecognizable-model@arbitrary_instance")
            .unwrap();
        assert_eq!(model.identity.provider_profile_id, "openai");
        assert_eq!(model.identity.model_driver_id, "openai");
        assert_eq!(model.api_types, vec![ApiType::ImageTextToImage]);
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
        invalid.logical_mounts = vec!["audio.tts".to_owned()];
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

#[cfg(test)]
pub(crate) mod llm_tests;
