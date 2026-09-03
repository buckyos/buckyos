#![allow(dead_code)]

use crate::catalog::CatalogSnapshot;
use crate::model::{ModelRegistry, ProviderInventory as ModelProviderInventory};
use crate::protocol::ResolvedCredential;
use crate::provider::{
    ExecutableProviderInstance, ProviderInstanceConfig, ProviderInventorySnapshot, ProviderProfile,
    ProviderRefreshEvent, ProviderRefreshOutcome, ProviderRefreshTrigger, ProviderRegistry,
    ProviderResult, ProviderRuntimeManager,
};
use crate::settings::{AiccSettings, SettingsDocument, SettingsError};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderMetadataState {
    pub metadata_applied_seq: u64,
    pub metadata_updating_seq: Option<u64>,
    pub routable: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetadataStatus {
    pub target_seq: u64,
    pub providers: BTreeMap<String, ProviderMetadataState>,
}

#[derive(Debug)]
pub(crate) struct RuntimeSnapshot {
    pub generation: u64,
    pub settings_revision: u64,
    pub metadata_target_seq: u64,
    pub settings: Arc<AiccSettings>,
    pub catalog: Arc<CatalogSnapshot>,
    pub providers: Arc<RuntimeProviderRegistry>,
    pub models: Arc<ModelRegistry>,
    pub provider_metadata: BTreeMap<String, ProviderMetadataState>,
}

#[derive(Clone)]
pub(crate) struct RuntimeProvider {
    executable: Arc<ExecutableProviderInstance>,
    pub config: Arc<ProviderInstanceConfig>,
    pub profile: Arc<ProviderProfile>,
    pub inventory: Arc<ProviderInventorySnapshot>,
}

impl RuntimeProvider {
    pub(crate) async fn resolve_credential(&self) -> ProviderResult<ResolvedCredential> {
        self.executable.resolve_credential().await
    }
}

impl std::fmt::Debug for RuntimeProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeProvider")
            .field(
                "provider_instance_name",
                &self.inventory.provider_instance_name,
            )
            .field("metadata_applied_seq", &self.inventory.metadata_applied_seq)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RuntimeProviderRegistry {
    instances: BTreeMap<String, RuntimeProvider>,
}

impl RuntimeProviderRegistry {
    pub(crate) async fn capture(providers: &ProviderRegistry) -> Self {
        let mut instances = BTreeMap::new();
        for executable in providers.list() {
            let inventory = executable.current_inventory().await;
            instances.insert(
                executable.config.provider_instance_name.clone(),
                RuntimeProvider {
                    config: executable.config.clone(),
                    profile: executable.profile.clone(),
                    executable,
                    inventory,
                },
            );
        }
        Self { instances }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&RuntimeProvider> {
        self.instances.get(name)
    }

    pub(crate) fn list(&self) -> Vec<&RuntimeProvider> {
        self.instances.values().collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConvergenceTrigger {
    Inference,
    ProviderRefresh {
        provider_instance_name: String,
        inventory_changed: bool,
    },
    MetadataRefresh,
}

impl ConvergenceTrigger {
    fn is_probe(&self) -> bool {
        matches!(self, Self::ProviderRefresh { .. })
    }

    fn provider_inventory_changed(&self) -> bool {
        matches!(
            self,
            Self::ProviderRefresh {
                inventory_changed: true,
                ..
            }
        )
    }
}

#[derive(Clone)]
pub(crate) struct RuntimePreparedState {
    pub catalog: Arc<CatalogSnapshot>,
    pub providers: Arc<RuntimeProviderRegistry>,
    pub models: Arc<ModelRegistry>,
    pub provider_metadata: BTreeMap<String, ProviderMetadataState>,
    pub changed: bool,
}

#[async_trait]
pub(crate) trait RuntimeBackend: Send + Sync {
    async fn converge(
        &self,
        catalog: Arc<CatalogSnapshot>,
        target_seq: u64,
        trigger: ConvergenceTrigger,
    ) -> Result<RuntimePreparedState, RuntimeError>;

    async fn shutdown(&self);
}

#[async_trait]
pub(crate) trait ModelRegistryAssembler: Send + Sync {
    async fn build(
        &self,
        catalog: Arc<CatalogSnapshot>,
        inventories: Vec<ModelProviderInventory>,
    ) -> Result<Arc<ModelRegistry>, RuntimeError>;
}

pub(crate) struct ProviderRuntimeBackend {
    manager: Arc<ProviderRuntimeManager>,
    models: Arc<dyn ModelRegistryAssembler>,
}

impl ProviderRuntimeBackend {
    pub(crate) fn new(
        manager: Arc<ProviderRuntimeManager>,
        models: Arc<dyn ModelRegistryAssembler>,
    ) -> Self {
        Self { manager, models }
    }

    pub(crate) fn manager(&self) -> &Arc<ProviderRuntimeManager> {
        &self.manager
    }
}

#[async_trait]
impl RuntimeBackend for ProviderRuntimeBackend {
    async fn converge(
        &self,
        catalog: Arc<CatalogSnapshot>,
        target_seq: u64,
        trigger: ConvergenceTrigger,
    ) -> Result<RuntimePreparedState, RuntimeError> {
        let before_catalog_seq = self.manager.current_catalog().await.target_revision_seq();
        let before_registry = self.manager.registry().await;
        let mut has_lagging_provider = false;
        for instance in before_registry.list() {
            if instance.current_inventory().await.metadata_applied_seq != target_seq {
                has_lagging_provider = true;
                break;
            }
        }
        let must_reconcile = before_catalog_seq != target_seq || has_lagging_provider;
        let reports = if must_reconcile {
            self.manager.reconcile_inventory(catalog.clone()).await
        } else {
            Vec::new()
        };
        let failures: BTreeMap<_, _> = reports
            .iter()
            .filter_map(|event| match &event.outcome {
                ProviderRefreshOutcome::Failed { kind } => {
                    Some((event.provider_instance_name.clone(), format!("{kind:?}")))
                }
                ProviderRefreshOutcome::Committed { .. } => None,
            })
            .collect();
        let provider_registry = self.manager.registry().await;
        let providers = Arc::new(RuntimeProviderRegistry::capture(&provider_registry).await);
        let mut provider_metadata = BTreeMap::new();
        let mut inventories = Vec::new();
        for instance in providers.list() {
            let inventory = &instance.inventory;
            let name = inventory.provider_instance_name.clone();
            let last_error = failures.get(&name).cloned();
            let routable = inventory.metadata_applied_seq == target_seq && last_error.is_none();
            provider_metadata.insert(
                name,
                ProviderMetadataState {
                    metadata_applied_seq: inventory.metadata_applied_seq,
                    metadata_updating_seq: None,
                    routable,
                    last_error,
                },
            );
            if routable {
                inventories.push(inventory.as_model_inventory());
            }
        }
        let models = self.models.build(catalog.clone(), inventories).await?;
        Ok(RuntimePreparedState {
            catalog,
            providers,
            models,
            provider_metadata,
            changed: must_reconcile || trigger.provider_inventory_changed(),
        })
    }

    async fn shutdown(&self) {
        self.manager.shutdown().await;
    }
}

pub(crate) struct PreparedRuntime {
    pub backend: Arc<dyn RuntimeBackend>,
    pub state: RuntimePreparedState,
}

#[async_trait]
pub(crate) trait RuntimeFactory: Send + Sync {
    async fn prepare(
        &self,
        settings: Arc<AiccSettings>,
        catalog: Arc<CatalogSnapshot>,
        target_seq: u64,
    ) -> Result<PreparedRuntime, RuntimeError>;
}

#[async_trait]
pub(crate) trait RuntimeInputs: Send + Sync {
    async fn metadata_target_seq(&self) -> Result<u64, RuntimeError>;

    async fn load_catalog(&self, target_seq: u64) -> Result<Arc<CatalogSnapshot>, RuntimeError>;
}

struct RuntimeGeneration {
    snapshot: Arc<RuntimeSnapshot>,
    backend: Arc<dyn RuntimeBackend>,
}

pub(crate) struct RuntimeState {
    current: RwLock<Arc<RuntimeGeneration>>,
    inputs: Arc<dyn RuntimeInputs>,
    factory: Arc<dyn RuntimeFactory>,
    convergence: Mutex<()>,
    observed_target_seq: AtomicU64,
    updating: Arc<StdRwLock<BTreeMap<String, u64>>>,
    stopped: AtomicBool,
}

struct UpdatingGuard {
    state: Arc<StdRwLock<BTreeMap<String, u64>>>,
}

impl Drop for UpdatingGuard {
    fn drop(&mut self) {
        write_std_lock(&self.state).clear();
    }
}

impl RuntimeState {
    pub(crate) async fn bootstrap(
        settings: SettingsDocument,
        inputs: Arc<dyn RuntimeInputs>,
        factory: Arc<dyn RuntimeFactory>,
    ) -> Result<Arc<Self>, RuntimeError> {
        let target_seq = inputs.metadata_target_seq().await?;
        let catalog = inputs.load_catalog(target_seq).await?;
        let prepared = factory
            .prepare(settings.settings.clone(), catalog, target_seq)
            .await?;
        let snapshot = match build_snapshot(1, &settings, target_seq, prepared.state) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                prepared.backend.shutdown().await;
                return Err(error);
            }
        };
        Ok(Arc::new(Self {
            current: RwLock::new(Arc::new(RuntimeGeneration {
                snapshot,
                backend: prepared.backend,
            })),
            inputs,
            factory,
            convergence: Mutex::new(()),
            observed_target_seq: AtomicU64::new(target_seq),
            updating: Arc::new(StdRwLock::new(BTreeMap::new())),
            stopped: AtomicBool::new(false),
        }))
    }

    pub(crate) async fn capture(&self) -> Arc<RuntimeSnapshot> {
        self.current.read().await.snapshot.clone()
    }

    pub(crate) async fn reload(
        &self,
        settings: SettingsDocument,
    ) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
        let _single_executor = self.convergence.lock().await;
        self.ensure_running()?;
        let current_generation = self.current.read().await.clone();
        let target_seq = self.inputs.metadata_target_seq().await?;
        self.observed_target_seq
            .store(target_seq, Ordering::Release);
        let catalog = self.inputs.load_catalog(target_seq).await?;
        let prepared = self
            .factory
            .prepare(settings.settings.clone(), catalog, target_seq)
            .await?;
        if Arc::ptr_eq(&current_generation.backend, &prepared.backend) {
            return Err(RuntimeError::CandidateReusesBackend);
        }
        let next_generation = current_generation.snapshot.generation.saturating_add(1);
        let snapshot = match build_snapshot(next_generation, &settings, target_seq, prepared.state)
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                prepared.backend.shutdown().await;
                return Err(error);
            }
        };
        let replacement = Arc::new(RuntimeGeneration {
            snapshot: snapshot.clone(),
            backend: prepared.backend,
        });
        let previous = {
            let mut current = self.current.write().await;
            std::mem::replace(&mut *current, replacement)
        };
        previous.backend.shutdown().await;
        Ok(snapshot)
    }

    pub(crate) async fn before_inference(&self) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
        self.converge(ConvergenceTrigger::Inference).await
    }

    pub(crate) async fn provider_refreshed(
        &self,
        event: &ProviderRefreshEvent,
    ) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
        let current = self.capture().await;
        if matches!(event.trigger, ProviderRefreshTrigger::Reconciliation) {
            return Ok(current);
        }
        if !current
            .provider_metadata
            .contains_key(&event.provider_instance_name)
        {
            return Ok(current);
        }
        let inventory_changed = matches!(
            event.outcome,
            ProviderRefreshOutcome::Committed { changed: true, .. }
        );
        self.converge(ConvergenceTrigger::ProviderRefresh {
            provider_instance_name: event.provider_instance_name.clone(),
            inventory_changed,
        })
        .await
    }

    pub(crate) async fn metadata_refreshed(&self) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
        self.converge(ConvergenceTrigger::MetadataRefresh).await
    }

    async fn converge(
        &self,
        trigger: ConvergenceTrigger,
    ) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
        let observed_generation = self.capture().await.generation;
        let _single_executor = self.convergence.lock().await;
        self.ensure_running()?;
        let current_generation = self.current.read().await.clone();
        let current = current_generation.snapshot.clone();
        let target_seq = self.inputs.metadata_target_seq().await?;
        self.observed_target_seq
            .store(target_seq, Ordering::Release);
        if target_seq < current.metadata_target_seq {
            return Err(RuntimeError::MetadataRollback {
                current: current.metadata_target_seq,
                observed: target_seq,
            });
        }
        let lagging = lagging_providers(&current, target_seq);
        let catalog_behind = target_seq != current.metadata_target_seq;
        let concurrent_run_completed = current.generation != observed_generation;
        if !catalog_behind
            && lagging.is_empty()
            && (!trigger.is_probe() || concurrent_run_completed)
        {
            return Ok(current);
        }
        {
            *write_std_lock(&self.updating) = lagging
                .iter()
                .map(|name| (name.clone(), target_seq))
                .collect();
        }
        let _updating_guard = UpdatingGuard {
            state: self.updating.clone(),
        };
        let prepared = async {
            let catalog = if target_seq == current.metadata_target_seq {
                current.catalog.clone()
            } else {
                self.inputs.load_catalog(target_seq).await?
            };
            current_generation
                .backend
                .converge(catalog, target_seq, trigger)
                .await
        }
        .await;
        let prepared = prepared?;
        if !catalog_behind && lagging.is_empty() && !prepared.changed {
            return Ok(current);
        }
        let settings = SettingsDocument {
            revision: current.settings_revision,
            settings: current.settings.clone(),
        };
        let snapshot = build_snapshot(
            current.generation.saturating_add(1),
            &settings,
            target_seq,
            prepared,
        )?;
        let replacement = Arc::new(RuntimeGeneration {
            snapshot: snapshot.clone(),
            backend: current_generation.backend.clone(),
        });
        *self.current.write().await = replacement;
        Ok(snapshot)
    }

    pub(crate) async fn metadata_status(&self) -> MetadataStatus {
        let snapshot = self.capture().await;
        let updating = read_std_lock(&self.updating);
        let providers = snapshot
            .provider_metadata
            .iter()
            .map(|(name, state)| {
                let mut state = state.clone();
                state.metadata_updating_seq = updating.get(name).copied();
                (name.clone(), state)
            })
            .collect();
        MetadataStatus {
            target_seq: self.observed_target_seq.load(Ordering::Acquire),
            providers,
        }
    }

    pub(crate) async fn shutdown(&self) {
        let _single_executor = self.convergence.lock().await;
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.current.read().await.backend.shutdown().await;
        write_std_lock(&self.updating).clear();
    }

    fn ensure_running(&self) -> Result<(), RuntimeError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::Stopped);
        }
        Ok(())
    }
}

fn read_std_lock<T>(lock: &StdRwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_std_lock<T>(lock: &StdRwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lagging_providers(snapshot: &RuntimeSnapshot, target_seq: u64) -> BTreeSet<String> {
    snapshot
        .provider_metadata
        .iter()
        .filter(|(_, state)| state.metadata_applied_seq != target_seq)
        .map(|(name, _)| name.clone())
        .collect()
}

fn build_snapshot(
    generation: u64,
    settings: &SettingsDocument,
    target_seq: u64,
    state: RuntimePreparedState,
) -> Result<Arc<RuntimeSnapshot>, RuntimeError> {
    if state.catalog.target_revision_seq() != target_seq {
        return Err(RuntimeError::CandidateCatalogMismatch {
            expected: target_seq,
            actual: state.catalog.target_revision_seq(),
        });
    }
    let expected = settings.settings.enabled_provider_names();
    let actual: BTreeSet<_> = state.provider_metadata.keys().cloned().collect();
    if expected != actual {
        return Err(RuntimeError::CandidateProviderSetMismatch { expected, actual });
    }
    for (name, provider) in &state.provider_metadata {
        if provider.metadata_updating_seq.is_some() {
            return Err(RuntimeError::CandidateStillUpdating(name.clone()));
        }
        if provider.routable && provider.metadata_applied_seq != target_seq {
            return Err(RuntimeError::MixedCatalogRevision {
                provider_instance_name: name.clone(),
                applied: provider.metadata_applied_seq,
                target: target_seq,
            });
        }
    }
    for model in state.models.model_views() {
        match state.provider_metadata.get(&model.provider_instance_name) {
            Some(provider) if provider.routable => {}
            _ => {
                return Err(RuntimeError::UnconvergedModel {
                    exact_model: model.exact_model,
                    provider_instance_name: model.provider_instance_name,
                });
            }
        }
    }
    Ok(Arc::new(RuntimeSnapshot {
        generation,
        settings_revision: settings.revision,
        metadata_target_seq: target_seq,
        settings: settings.settings.clone(),
        catalog: state.catalog,
        providers: state.providers,
        models: state.models,
        provider_metadata: state.provider_metadata,
    }))
}

#[derive(Debug, Error)]
pub(crate) enum RuntimeError {
    #[error("AICC runtime has stopped")]
    Stopped,
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("runtime backend failed: {0}")]
    Backend(String),
    #[error("metadata target sequence moved backwards from {current} to {observed}")]
    MetadataRollback { current: u64, observed: u64 },
    #[error("candidate catalog sequence is {actual}, expected {expected}")]
    CandidateCatalogMismatch { expected: u64, actual: u64 },
    #[error("reload candidate reused the currently published runtime backend")]
    CandidateReusesBackend,
    #[error("candidate provider set differs from enabled settings")]
    CandidateProviderSetMismatch {
        expected: BTreeSet<String>,
        actual: BTreeSet<String>,
    },
    #[error("candidate provider `{0}` still has metadata_updating_seq")]
    CandidateStillUpdating(String),
    #[error(
        "candidate provider `{provider_instance_name}` is routable at applied seq {applied}, target is {target}"
    )]
    MixedCatalogRevision {
        provider_instance_name: String,
        applied: u64,
        target: u64,
    },
    #[error("model `{exact_model}` references non-routable provider `{provider_instance_name}`")]
    UnconvergedModel {
        exact_model: String,
        provider_instance_name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogDocuments};
    use crate::model::RegistryLayers;
    use crate::provider::{ProviderRefreshFailure, ProviderRefreshOutcome};
    use crate::settings::ProviderSettings;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::Notify;

    fn settings(revision: u64, names: &[&str]) -> SettingsDocument {
        SettingsDocument::new(
            revision,
            AiccSettings {
                providers: names
                    .iter()
                    .map(|name| ProviderSettings {
                        provider_instance_name: (*name).into(),
                        provider_type: "cloud_api".into(),
                        provider_profile_id: "openai".into(),
                        protocol_adapter_id: "openai-responses".into(),
                        base_url: "https://api.example/v1".into(),
                        credentials: json!({"credential_ref": format!("secret://{name}")}),
                        enabled: true,
                        region: None,
                        account: None,
                        provider_rules_id: Some("openai".into()),
                        auth: None,
                        discovery: None,
                        instance_rules: None,
                        timeout_ms: None,
                        auto_sync_models: None,
                    })
                    .collect(),
                session_config: None,
            },
        )
        .unwrap()
    }

    fn catalog(seq: u64) -> Arc<CatalogSnapshot> {
        Arc::new(
            CatalogSnapshot::build(
                seq,
                CatalogDocuments::default(),
                &CatalogBuildOptions::default(),
            )
            .unwrap(),
        )
    }

    fn empty_models(catalog: &CatalogSnapshot) -> Arc<ModelRegistry> {
        Arc::new(ModelRegistry::build(catalog, &[], vec![], RegistryLayers::default()).unwrap())
    }

    fn prepared_state(seq: u64, names: &[&str], changed: bool) -> RuntimePreparedState {
        let catalog = catalog(seq);
        RuntimePreparedState {
            models: empty_models(&catalog),
            catalog,
            providers: Arc::new(RuntimeProviderRegistry::default()),
            provider_metadata: names
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        ProviderMetadataState {
                            metadata_applied_seq: seq,
                            metadata_updating_seq: None,
                            routable: true,
                            last_error: None,
                        },
                    )
                })
                .collect(),
            changed,
        }
    }

    struct TestInputs {
        target: AtomicU64,
    }

    #[async_trait]
    impl RuntimeInputs for TestInputs {
        async fn metadata_target_seq(&self) -> Result<u64, RuntimeError> {
            Ok(self.target.load(Ordering::SeqCst))
        }

        async fn load_catalog(
            &self,
            target_seq: u64,
        ) -> Result<Arc<CatalogSnapshot>, RuntimeError> {
            Ok(catalog(target_seq))
        }
    }

    struct TestBackend {
        calls: AtomicU64,
        shutdowns: AtomicU64,
        provider_names: Vec<String>,
        fail_provider: Option<String>,
        started: Option<Arc<Notify>>,
        proceed: Option<Arc<Notify>>,
    }

    #[async_trait]
    impl RuntimeBackend for TestBackend {
        async fn converge(
            &self,
            catalog: Arc<CatalogSnapshot>,
            target_seq: u64,
            _trigger: ConvergenceTrigger,
        ) -> Result<RuntimePreparedState, RuntimeError> {
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0 {
                if let Some(started) = &self.started {
                    started.notify_one();
                }
                if let Some(proceed) = &self.proceed {
                    proceed.notified().await;
                }
            }
            let provider_metadata = self
                .provider_names
                .iter()
                .map(|name| {
                    let failed = self.fail_provider.as_ref() == Some(name);
                    (
                        name.clone(),
                        ProviderMetadataState {
                            metadata_applied_seq: if failed {
                                target_seq.saturating_sub(1)
                            } else {
                                target_seq
                            },
                            metadata_updating_seq: None,
                            routable: !failed,
                            last_error: failed.then(|| "discovery failed".into()),
                        },
                    )
                })
                .collect();
            Ok(RuntimePreparedState {
                models: empty_models(&catalog),
                catalog,
                providers: Arc::new(RuntimeProviderRegistry::default()),
                provider_metadata,
                changed: true,
            })
        }

        async fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct TestFactory {
        backend: Arc<TestBackend>,
    }

    #[async_trait]
    impl RuntimeFactory for TestFactory {
        async fn prepare(
            &self,
            settings: Arc<AiccSettings>,
            catalog: Arc<CatalogSnapshot>,
            target_seq: u64,
        ) -> Result<PreparedRuntime, RuntimeError> {
            let names: Vec<_> = settings.enabled_provider_names().into_iter().collect();
            let borrowed: Vec<_> = names.iter().map(String::as_str).collect();
            let mut state = prepared_state(target_seq, &borrowed, true);
            state.models = empty_models(&catalog);
            state.catalog = catalog;
            let configured_names: BTreeSet<_> = names.iter().cloned().collect();
            let backend_names: BTreeSet<_> = self.backend.provider_names.iter().cloned().collect();
            let backend: Arc<dyn RuntimeBackend> = if configured_names == backend_names {
                self.backend.clone()
            } else {
                Arc::new(TestBackend {
                    calls: AtomicU64::new(0),
                    shutdowns: AtomicU64::new(0),
                    provider_names: names,
                    fail_provider: None,
                    started: None,
                    proceed: None,
                })
            };
            Ok(PreparedRuntime { backend, state })
        }
    }

    async fn runtime(
        inputs: Arc<TestInputs>,
        backend: Arc<TestBackend>,
        names: &[&str],
    ) -> Arc<RuntimeState> {
        RuntimeState::bootstrap(
            settings(1, names),
            inputs,
            Arc::new(TestFactory { backend }),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn requests_capture_complete_old_or_new_generation() {
        struct BlockingReloadFactory {
            initial: Arc<TestBackend>,
            replacement: Arc<TestBackend>,
            calls: AtomicU64,
            started: Arc<Notify>,
            proceed: Arc<Notify>,
        }
        #[async_trait]
        impl RuntimeFactory for BlockingReloadFactory {
            async fn prepare(
                &self,
                settings: Arc<AiccSettings>,
                catalog: Arc<CatalogSnapshot>,
                target_seq: u64,
            ) -> Result<PreparedRuntime, RuntimeError> {
                let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
                if !first {
                    self.started.notify_one();
                    self.proceed.notified().await;
                }
                let names: Vec<_> = settings.enabled_provider_names().into_iter().collect();
                let borrowed: Vec<_> = names.iter().map(String::as_str).collect();
                let mut state = prepared_state(target_seq, &borrowed, true);
                state.catalog = catalog;
                Ok(PreparedRuntime {
                    backend: if first {
                        self.initial.clone()
                    } else {
                        self.replacement.clone()
                    },
                    state,
                })
            }
        }

        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let replacement = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["replacement".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let started = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let runtime = RuntimeState::bootstrap(
            settings(1, &["primary"]),
            inputs,
            Arc::new(BlockingReloadFactory {
                initial: backend.clone(),
                replacement,
                calls: AtomicU64::new(0),
                started: started.clone(),
                proceed: proceed.clone(),
            }),
        )
        .await
        .unwrap();
        let old = runtime.capture().await;
        let reload_runtime = runtime.clone();
        let reload = tokio::spawn(async move {
            reload_runtime
                .reload(settings(2, &["replacement"]))
                .await
                .unwrap()
        });
        started.notified().await;
        let during_prepare = runtime.capture().await;
        assert!(Arc::ptr_eq(&old, &during_prepare));
        proceed.notify_one();
        let new = reload.await.unwrap();
        assert_eq!(old.generation, 1);
        assert_eq!(old.settings.providers[0].provider_instance_name, "primary");
        assert_eq!(new.generation, 2);
        assert_eq!(
            new.settings.providers[0].provider_instance_name,
            "replacement"
        );
        assert_eq!(backend.shutdowns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalid_reload_candidate_is_retired_without_replacing_current_snapshot() {
        struct InvalidReloadFactory {
            initial: Arc<TestBackend>,
            candidate: Arc<TestBackend>,
            calls: AtomicU64,
        }
        #[async_trait]
        impl RuntimeFactory for InvalidReloadFactory {
            async fn prepare(
                &self,
                settings: Arc<AiccSettings>,
                catalog: Arc<CatalogSnapshot>,
                target_seq: u64,
            ) -> Result<PreparedRuntime, RuntimeError> {
                let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
                let names: Vec<_> = settings.enabled_provider_names().into_iter().collect();
                let borrowed: Vec<_> = names.iter().map(String::as_str).collect();
                let mut state = prepared_state(target_seq, &borrowed, true);
                state.catalog = catalog;
                if !first {
                    state.provider_metadata.clear();
                }
                Ok(PreparedRuntime {
                    backend: if first {
                        self.initial.clone()
                    } else {
                        self.candidate.clone()
                    },
                    state,
                })
            }
        }

        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let initial = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let candidate = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["replacement".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let runtime = RuntimeState::bootstrap(
            settings(1, &["primary"]),
            inputs,
            Arc::new(InvalidReloadFactory {
                initial: initial.clone(),
                candidate: candidate.clone(),
                calls: AtomicU64::new(0),
            }),
        )
        .await
        .unwrap();
        let before = runtime.capture().await;
        assert!(matches!(
            runtime.reload(settings(2, &["replacement"])).await,
            Err(RuntimeError::CandidateProviderSetMismatch { .. })
        ));
        let after = runtime.capture().await;
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(initial.shutdowns.load(Ordering::SeqCst), 0);
        assert_eq!(candidate.shutdowns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_convergence_has_one_executor_and_exposes_updating_seq() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let started = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: Some(started.clone()),
            proceed: Some(proceed.clone()),
        });
        let runtime = runtime(inputs.clone(), backend.clone(), &["primary"]).await;
        inputs.target.store(2, Ordering::SeqCst);
        let first_runtime = runtime.clone();
        let first = tokio::spawn(async move { first_runtime.before_inference().await.unwrap() });
        started.notified().await;
        let status = runtime.metadata_status().await;
        assert_eq!(status.target_seq, 2);
        assert_eq!(status.providers["primary"].metadata_updating_seq, Some(2));
        let second_runtime = runtime.clone();
        let second = tokio::spawn(async move { second_runtime.before_inference().await.unwrap() });
        proceed.notify_waiters();
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap().generation, 2);
        assert_eq!(second.unwrap().generation, 2);
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn target_advance_during_refresh_commits_only_captured_sequence() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let started = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: Some(started.clone()),
            proceed: Some(proceed.clone()),
        });
        let runtime = runtime(inputs.clone(), backend, &["primary"]).await;
        inputs.target.store(2, Ordering::SeqCst);
        let task_runtime = runtime.clone();
        let task = tokio::spawn(async move { task_runtime.before_inference().await.unwrap() });
        started.notified().await;
        inputs.target.store(3, Ordering::SeqCst);
        proceed.notify_waiters();
        let first = task.await.unwrap();
        assert_eq!(first.metadata_target_seq, 2);
        let second = runtime.before_inference().await.unwrap();
        assert_eq!(second.metadata_target_seq, 3);
    }

    #[tokio::test]
    async fn provider_failure_retains_applied_seq_and_is_not_routable() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["good".into(), "failed".into()],
            fail_provider: Some("failed".into()),
            started: None,
            proceed: None,
        });
        let runtime = runtime(inputs.clone(), backend, &["good", "failed"]).await;
        inputs.target.store(2, Ordering::SeqCst);
        let snapshot = runtime.before_inference().await.unwrap();
        assert_eq!(snapshot.provider_metadata["good"].metadata_applied_seq, 2);
        assert_eq!(snapshot.provider_metadata["failed"].metadata_applied_seq, 1);
        assert!(!snapshot.provider_metadata["failed"].routable);
        assert_eq!(
            snapshot.provider_metadata["failed"].last_error.as_deref(),
            Some("discovery failed")
        );
    }

    #[tokio::test]
    async fn unchanged_provider_probe_does_not_publish_snapshot() {
        struct ProbeBackend(AtomicU64);
        #[async_trait]
        impl RuntimeBackend for ProbeBackend {
            async fn converge(
                &self,
                catalog: Arc<CatalogSnapshot>,
                target_seq: u64,
                _trigger: ConvergenceTrigger,
            ) -> Result<RuntimePreparedState, RuntimeError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                let mut state = prepared_state(target_seq, &["primary"], false);
                state.catalog = catalog;
                Ok(state)
            }
            async fn shutdown(&self) {}
        }
        struct ProbeFactory(Arc<ProbeBackend>);
        #[async_trait]
        impl RuntimeFactory for ProbeFactory {
            async fn prepare(
                &self,
                _settings: Arc<AiccSettings>,
                _catalog: Arc<CatalogSnapshot>,
                target_seq: u64,
            ) -> Result<PreparedRuntime, RuntimeError> {
                Ok(PreparedRuntime {
                    backend: self.0.clone(),
                    state: prepared_state(target_seq, &["primary"], true),
                })
            }
        }
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(ProbeBackend(AtomicU64::new(0)));
        let runtime = RuntimeState::bootstrap(
            settings(1, &["primary"]),
            inputs,
            Arc::new(ProbeFactory(backend.clone())),
        )
        .await
        .unwrap();
        let before = runtime.capture().await;
        let event = ProviderRefreshEvent {
            provider_instance_name: "primary".into(),
            trigger: ProviderRefreshTrigger::Scheduled,
            outcome: ProviderRefreshOutcome::Committed {
                changed: false,
                inventory_revision: Some("same".into()),
                metadata_applied_seq: 1,
            },
        };
        let after = runtime.provider_refreshed(&event).await.unwrap();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(backend.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reconciliation_events_do_not_recursively_trigger_convergence() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let runtime = runtime(inputs, backend.clone(), &["primary"]).await;
        let event = ProviderRefreshEvent {
            provider_instance_name: "primary".into(),
            trigger: ProviderRefreshTrigger::Reconciliation,
            outcome: ProviderRefreshOutcome::Failed {
                kind: ProviderRefreshFailure::Discovery,
            },
        };
        runtime.provider_refreshed(&event).await.unwrap();
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn metadata_target_advances_even_without_provider_instances() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec![],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let runtime = runtime(inputs.clone(), backend.clone(), &[]).await;
        inputs.target.store(2, Ordering::SeqCst);
        let snapshot = runtime.before_inference().await.unwrap();
        assert_eq!(snapshot.metadata_target_seq, 2);
        assert_eq!(snapshot.catalog.target_revision_seq(), 2);
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn late_refresh_from_removed_provider_is_ignored() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let runtime = runtime(inputs, backend.clone(), &["primary"]).await;
        runtime.reload(settings(2, &[])).await.unwrap();
        let event = ProviderRefreshEvent {
            provider_instance_name: "primary".into(),
            trigger: ProviderRefreshTrigger::Scheduled,
            outcome: ProviderRefreshOutcome::Committed {
                changed: true,
                inventory_revision: Some("late".into()),
                metadata_applied_seq: 1,
            },
        };
        let before = runtime.capture().await;
        let after = runtime.provider_refreshed(&event).await.unwrap();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancelled_convergence_clears_transient_updating_sequence() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let started = Arc::new(Notify::new());
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: Some(started.clone()),
            proceed: Some(Arc::new(Notify::new())),
        });
        let runtime = runtime(inputs.clone(), backend, &["primary"]).await;
        inputs.target.store(2, Ordering::SeqCst);
        let task_runtime = runtime.clone();
        let task = tokio::spawn(async move { task_runtime.before_inference().await });
        started.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            runtime.metadata_status().await.providers["primary"].metadata_updating_seq,
            None
        );
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_and_rejects_new_work() {
        let inputs = Arc::new(TestInputs {
            target: AtomicU64::new(1),
        });
        let backend = Arc::new(TestBackend {
            calls: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            provider_names: vec!["primary".into()],
            fail_provider: None,
            started: None,
            proceed: None,
        });
        let runtime = runtime(inputs, backend.clone(), &["primary"]).await;
        runtime.shutdown().await;
        runtime.shutdown().await;
        assert_eq!(backend.shutdowns.load(Ordering::SeqCst), 1);
        assert!(matches!(
            runtime.before_inference().await,
            Err(RuntimeError::Stopped)
        ));
        assert!(matches!(
            runtime.reload(settings(2, &[])).await,
            Err(RuntimeError::Stopped)
        ));
    }
}
