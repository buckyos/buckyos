use super::*;

pub(super) struct ProviderRuntime {
    config: Arc<ProviderInstanceConfig>,
    profile: Arc<ProviderProfile>,
    discovery: Arc<dyn ProviderDiscovery>,
    credential_resolver: Arc<dyn CredentialResolver>,
    catalog: Arc<RwLock<Arc<CatalogSnapshot>>>,
    codecs: Arc<CodecRegistry>,
    store: Arc<dyn ProviderInventoryStore>,
    registry: Arc<RwLock<Arc<ProviderRegistry>>>,
    refresh_events: broadcast::Sender<ProviderRefreshEvent>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    current_candidate_seq: AtomicU64,
    commit_gate: RwLock<()>,
    refresh_lock: Mutex<()>,
    inventory: RwLock<Arc<ProviderInventorySnapshot>>,
    pub(super) health: RwLock<ProviderHealth>,
    stopped: AtomicBool,
    stop_tx: watch::Sender<bool>,
    pub(super) task: Mutex<Option<JoinHandle<()>>>,
    stop_lock: Mutex<()>,
}

impl ProviderRuntime {
    pub(super) async fn resolve_credential(&self) -> ProviderResult<ResolvedCredential> {
        let descriptor = self.profile.credential_for(self.config.credential_kind)?;
        self.credential_resolver
            .resolve(descriptor, &self.config.credential)
            .await
    }

    pub(super) async fn quota_observation(&self) -> ProviderQuotaObservation {
        ProviderQuotaObservation {
            state: ProviderQuotaObservationState::Unsupported,
            remaining_request_units: None,
            remaining_cost: None,
            reset_at_ms: None,
            observed_at_ms: now_ms().unwrap_or(0),
            source: "unsupported".into(),
        }
    }

    async fn build_candidate(&self) -> ProviderResult<ProviderInventoryCandidate> {
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        let candidate_seq = self.current_candidate_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let catalog = self.catalog.read().await.clone();
        let credential = self.resolve_credential().await?;
        let snapshot = self
            .discovery
            .discover(&DiscoveryContext {
                profile: &self.profile,
                instance: &self.config,
                credential: &credential,
            })
            .await?;
        let inventory = InventoryBuilder::build(
            &self.profile,
            &self.config,
            snapshot,
            &catalog,
            &self.codecs,
        )?;
        Ok(ProviderInventoryCandidate {
            provider_instance_name: self.config.provider_instance_name.clone(),
            generation: self.generation,
            candidate_seq,
            catalog_revision_seq: catalog.target_revision_seq(),
            inventory: Arc::new(inventory),
        })
    }

    pub(super) async fn refresh_once(
        self: &Arc<Self>,
        force: bool,
        trigger: ProviderRefreshTrigger,
        publish: bool,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let _refresh = self.refresh_lock.lock().await;
        let attempt_at_ms = now_ms()?;
        let candidate = match self.build_candidate().await {
            Ok(built) => built,
            Err(error) => {
                self.record_failure(attempt_at_ms, &error).await;
                self.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                return Err(error);
            }
        };
        let current = self.inventory.read().await.clone();
        let changed = force
            || current.provider_model_list_fingerprint
                != candidate.inventory.provider_model_list_fingerprint
            || current.metadata_applied_seq != candidate.inventory.metadata_applied_seq;
        if changed {
            if let Err(error) = self.commit_candidate(&candidate, publish).await {
                if !matches!(error, ProviderError::StaleCandidate) {
                    self.record_failure(attempt_at_ms, &error).await;
                }
                self.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                return Err(error);
            }
        }
        self.record_success(attempt_at_ms, candidate.inventory.health)
            .await?;
        let inventory = if changed {
            candidate.inventory.clone()
        } else {
            current
        };
        self.publish_refresh_event(
            trigger,
            ProviderRefreshOutcome::Committed {
                changed,
                inventory_revision: inventory.inventory_revision.clone(),
                metadata_applied_seq: inventory.metadata_applied_seq,
            },
        );
        Ok(inventory)
    }

    async fn commit_candidate(
        self: &Arc<Self>,
        candidate: &ProviderInventoryCandidate,
        publish: bool,
    ) -> ProviderResult<()> {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        let latest_catalog_revision = self.catalog.read().await.target_revision_seq();
        if candidate.provider_instance_name != self.config.provider_instance_name
            || candidate.generation != self.generation
            || candidate.candidate_seq != self.current_candidate_seq.load(Ordering::Acquire)
            || candidate.catalog_revision_seq != latest_catalog_revision
        {
            return Err(ProviderError::StaleCandidate);
        }
        let inventory = candidate.inventory.clone();
        let snapshot = serde_json::to_value(inventory.as_ref())
            .map_err(|error| ProviderError::Inventory(error.to_string()))?;
        let record = InventoryLkgsRecord::new(
            &inventory.provider_instance_name,
            &inventory.provider_profile_id,
            &inventory.protocol_adapter_id,
            &inventory.provider_model_list_fingerprint,
            inventory.metadata_applied_seq,
            inventory.inventory_revision.clone(),
            inventory.discovered_at_ms,
            snapshot,
            now_ms()?,
        )
        .map_err(|error| ProviderError::Storage(error.to_string()))?;
        self.store.commit(&record).await?;
        *self.inventory.write().await = inventory.clone();
        if publish {
            let executable = Arc::new(ExecutableProviderInstance {
                config: self.config.clone(),
                profile: self.profile.clone(),
                inventory,
                generation: self.generation,
                runtime: self.clone(),
            });
            let current = self.registry.read().await.clone();
            let mut next = current.instances.clone();
            next.insert(self.config.provider_instance_name.clone(), executable);
            *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
        }
        Ok(())
    }

    fn publish_refresh_event(
        &self,
        trigger: ProviderRefreshTrigger,
        outcome: ProviderRefreshOutcome,
    ) {
        let _ = self.refresh_events.send(ProviderRefreshEvent {
            provider_instance_name: self.config.provider_instance_name.clone(),
            trigger,
            outcome,
        });
    }

    async fn record_success(&self, at_ms: i64, state: ProviderHealthState) -> ProviderResult<()> {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        *self.health.write().await = ProviderHealth {
            state,
            consecutive_failures: 0,
            last_success_at_ms: Some(at_ms),
            last_attempt_at_ms: Some(at_ms),
            last_error: None,
        };
        Ok(())
    }

    async fn record_failure(&self, at_ms: i64, error: &ProviderError) {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return;
        }
        let mut health = self.health.write().await;
        health.state = ProviderHealthState::Degraded;
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_attempt_at_ms = Some(at_ms);
        health.last_error = Some(error.to_string());
    }

    fn is_current(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.current_generation.load(Ordering::Acquire) == self.generation
    }

    async fn run(self: Arc<Self>, mut stop_rx: watch::Receiver<bool>) {
        let mut delay = self.profile.refresh.interval;
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(delay) => {
                    match self.refresh_once(false, ProviderRefreshTrigger::Scheduled, true).await {
                        Ok(_) => delay = self.profile.refresh.interval,
                        Err(ProviderError::Stopped) => break,
                        Err(_) => {
                            let failures = self.health.read().await.consecutive_failures;
                            delay = exponential_backoff(&self.profile.refresh, failures);
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        let _stop = self.stop_lock.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        {
            let _gate = self.commit_gate.write().await;
            self.stopped.store(true, Ordering::Release);
            self.current_generation.fetch_add(1, Ordering::AcqRel);
            let _ = self.stop_tx.send(true);
        }
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
        let mut health = self.health.write().await;
        health.state = ProviderHealthState::Stopped;
        health.last_error = None;
    }
}

pub(crate) struct ProviderRuntimeManager {
    profiles: BTreeMap<String, Arc<ProviderProfile>>,
    credential_resolver: Arc<dyn CredentialResolver>,
    catalog: Arc<RwLock<Arc<CatalogSnapshot>>>,
    codecs: Arc<CodecRegistry>,
    store: Arc<dyn ProviderInventoryStore>,
    pub(super) runtimes: Mutex<BTreeMap<String, Arc<ProviderRuntime>>>,
    registry: Arc<RwLock<Arc<ProviderRegistry>>>,
    refresh_events: broadcast::Sender<ProviderRefreshEvent>,
    generations: Mutex<BTreeMap<String, Arc<AtomicU64>>>,
    lifecycle_lock: Mutex<()>,
}

impl ProviderRuntimeManager {
    pub(crate) fn new(
        profiles: impl IntoIterator<Item = ProviderProfile>,
        credential_resolver: Arc<dyn CredentialResolver>,
        catalog: Arc<CatalogSnapshot>,
        codecs: Arc<CodecRegistry>,
        store: Arc<dyn ProviderInventoryStore>,
    ) -> ProviderResult<Self> {
        let mut profile_map = BTreeMap::new();
        for profile in profiles {
            profile.validate()?;
            let id = profile.provider_profile_id.clone();
            if profile_map.insert(id.clone(), Arc::new(profile)).is_some() {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "duplicate provider profile `{id}`"
                )));
            }
        }
        let (refresh_events, _) = broadcast::channel(64);
        Ok(Self {
            profiles: profile_map,
            credential_resolver,
            catalog: Arc::new(RwLock::new(catalog)),
            codecs,
            store,
            runtimes: Mutex::new(BTreeMap::new()),
            registry: Arc::new(RwLock::new(Arc::new(ProviderRegistry::default()))),
            refresh_events,
            generations: Mutex::new(BTreeMap::new()),
            lifecycle_lock: Mutex::new(()),
        })
    }

    pub(crate) async fn registry(&self) -> Arc<ProviderRegistry> {
        self.registry.read().await.clone()
    }

    pub(crate) async fn quota_observation(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<ProviderQuotaObservation> {
        Ok(self
            .runtime(provider_instance_name)
            .await?
            .quota_observation()
            .await)
    }

    pub(crate) fn subscribe_refresh_events(&self) -> broadcast::Receiver<ProviderRefreshEvent> {
        self.refresh_events.subscribe()
    }

    pub(crate) async fn current_catalog(&self) -> Arc<CatalogSnapshot> {
        self.catalog.read().await.clone()
    }

    pub(crate) async fn validate_draft(
        &self,
        draft: &ProviderDraftConfig,
        connection_contract: &ProviderConnectionContract,
        discovery: &dyn ProviderDiscovery,
        dynamic_login_resolver: Option<&dyn DynamicLoginCredentialResolver>,
    ) -> Result<ProviderDraftNegotiation, ProviderDraftValidationError> {
        let profile = self
            .profiles
            .get(&draft.provider_profile_id)
            .cloned()
            .ok_or_else(|| ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::UnknownDependency,
            })?;
        if profile.default_protocol_adapter_id != draft.protocol_adapter_id
            && !profile.accepts_any_adapter
        {
            return Err(ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::InvalidConfiguration,
            });
        }
        if self.codecs.adapter(&draft.protocol_adapter_id).is_none() {
            return Err(ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::UnknownDependency,
            });
        }
        let connection = connection_contract
            .resolve(ProviderConnectionInput {
                base_url: draft.base_url.as_deref(),
                region: draft.region.as_deref(),
                workspace: draft.workspace.as_deref(),
                account: draft.account.as_deref(),
            })
            .map_err(|error| {
                ProviderDraftValidationError::from_provider_error(
                    ProviderDraftValidationStage::Connection,
                    &error,
                )
            })?;
        draft.auth.validate().map_err(|error| {
            ProviderDraftValidationError::from_provider_error(
                ProviderDraftValidationStage::Authentication,
                &error,
            )
        })?;
        let profile = Arc::new(
            profile
                .with_credential(draft.auth.credential_kind())
                .map_err(|error| {
                    ProviderDraftValidationError::from_provider_error(
                        ProviderDraftValidationStage::Authentication,
                        &error,
                    )
                })?,
        );
        let (credential_reference, credential) = match &draft.auth {
            ProviderAuthConfig::ApiKey {
                credential_ref,
                credential_kind,
            } => {
                let reference = CredentialReference {
                    reference: credential_ref.clone(),
                };
                let descriptor = profile.credential_for(*credential_kind).map_err(|error| {
                    ProviderDraftValidationError::from_provider_error(
                        ProviderDraftValidationStage::Authentication,
                        &error,
                    )
                })?;
                let credential = self
                    .credential_resolver
                    .resolve(descriptor, &reference)
                    .await
                    .map_err(|error| {
                        ProviderDraftValidationError::from_provider_error(
                            ProviderDraftValidationStage::Authentication,
                            &error,
                        )
                    })?;
                (reference, credential)
            }
            ProviderAuthConfig::DynamicLogin { .. } => {
                let user_name = draft.dynamic_login_user_name.as_deref().ok_or(
                    ProviderDraftValidationError {
                        stage: ProviderDraftValidationStage::Authentication,
                        kind: ProviderRefreshFailure::InvalidConfiguration,
                    },
                )?;
                let context = draft
                    .auth
                    .dynamic_login_context(&draft.provider_instance_name, user_name)
                    .map_err(|error| {
                        ProviderDraftValidationError::from_provider_error(
                            ProviderDraftValidationStage::Authentication,
                            &error,
                        )
                    })?;
                let resolver = dynamic_login_resolver.ok_or(ProviderDraftValidationError {
                    stage: ProviderDraftValidationStage::Authentication,
                    kind: ProviderRefreshFailure::UnknownDependency,
                })?;
                let credential = resolver.resolve_dynamic(&context).await.map_err(|error| {
                    ProviderDraftValidationError::from_provider_error(
                        ProviderDraftValidationStage::Authentication,
                        &error,
                    )
                })?;
                (
                    CredentialReference {
                        reference: "dynamic-login".into(),
                    },
                    credential,
                )
            }
        };
        let instance = ProviderInstanceConfig {
            provider_instance_name: draft.provider_instance_name.clone(),
            provider_profile_id: draft.provider_profile_id.clone(),
            protocol_adapter_id: draft.protocol_adapter_id.clone(),
            base_url: connection.base_url.clone(),
            credential: credential_reference,
            credential_kind: draft.auth.credential_kind(),
            provider_rules_id: draft.provider_rules_id.clone(),
            region: connection.region.clone(),
            workspace: connection.workspace.clone(),
            account: connection.account.clone(),
            request_timeout: Duration::from_secs(120),
            auto_sync_models: false,
            instance_rules: None,
        };
        instance.validate().map_err(|error| {
            ProviderDraftValidationError::from_provider_error(
                ProviderDraftValidationStage::Connection,
                &error,
            )
        })?;
        let discovered = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .map_err(|error| {
                ProviderDraftValidationError::from_provider_error(
                    ProviderDraftValidationStage::Discovery,
                    &error,
                )
            })?;
        let catalog = self.catalog.read().await.clone();
        let inventory =
            InventoryBuilder::build(&profile, &instance, discovered, &catalog, &self.codecs)
                .map_err(|error| {
                    let stage = if matches!(error, ProviderError::Discovery(_)) {
                        ProviderDraftValidationStage::Discovery
                    } else {
                        ProviderDraftValidationStage::Inventory
                    };
                    ProviderDraftValidationError::from_provider_error(stage, &error)
                })?;
        Ok(ProviderDraftNegotiation {
            provider_profile_id: profile.provider_profile_id.clone(),
            protocol_adapter_id: instance.protocol_adapter_id,
            auth_mode: draft.auth.mode(),
            connection,
            catalog_revision_seq: catalog.target_revision_seq(),
            inventory: Arc::new(inventory),
        })
    }

    pub(crate) async fn start(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self
            .runtimes
            .lock()
            .await
            .contains_key(&config.provider_instance_name)
        {
            return Err(ProviderError::DuplicateInstance(
                config.provider_instance_name,
            ));
        }
        self.start_unpublished(config, discovery).await
    }

    async fn start_unpublished(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        config.validate()?;
        let profile = self
            .profiles
            .get(&config.provider_profile_id)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownProfile(config.provider_profile_id.clone()))?;
        let profile = if profile.accepts_any_adapter {
            let adapter = self
                .codecs
                .adapter(&config.protocol_adapter_id)
                .ok_or_else(|| ProviderError::UnknownAdapter(config.protocol_adapter_id.clone()))?;
            crate::provider::builtin::custom_profile_for_adapter(
                profile.as_ref(),
                adapter,
                config.credential_kind,
            )?
        } else {
            profile.with_credential(config.credential_kind)?
        };
        let profile = Arc::new(profile);
        if profile.default_protocol_adapter_id != config.protocol_adapter_id
            && !profile.accepts_any_adapter
        {
            return Err(ProviderError::InvalidConfiguration(
                "dedicated provider must use its profile adapter".into(),
            ));
        }
        if self.codecs.adapter(&config.protocol_adapter_id).is_none() {
            return Err(ProviderError::UnknownAdapter(config.protocol_adapter_id));
        }
        let config = Arc::new(config);
        let generation_cell = {
            let mut generations = self.generations.lock().await;
            generations
                .entry(config.provider_instance_name.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        let generation = generation_cell.fetch_add(1, Ordering::AcqRel) + 1;
        let (stop_tx, stop_rx) = watch::channel(false);
        let catalog = self.catalog.read().await.clone();
        let placeholder = empty_inventory(&profile, &config, catalog.target_revision_seq());
        let runtime = Arc::new(ProviderRuntime {
            config: config.clone(),
            profile: profile.clone(),
            discovery,
            credential_resolver: self.credential_resolver.clone(),
            catalog: self.catalog.clone(),
            codecs: self.codecs.clone(),
            store: self.store.clone(),
            registry: self.registry.clone(),
            refresh_events: self.refresh_events.clone(),
            generation,
            current_generation: generation_cell,
            current_candidate_seq: AtomicU64::new(0),
            commit_gate: RwLock::new(()),
            refresh_lock: Mutex::new(()),
            inventory: RwLock::new(Arc::new(placeholder)),
            health: RwLock::new(ProviderHealth::default()),
            stopped: AtomicBool::new(false),
            stop_tx,
            task: Mutex::new(None),
            stop_lock: Mutex::new(()),
        });

        let inventory = match runtime
            .refresh_once(true, ProviderRefreshTrigger::Initial, false)
            .await
        {
            Ok(inventory) => inventory,
            Err(discovery_error) => match load_lkgs(&runtime).await {
                Ok(Some(inventory)) => inventory,
                Ok(None) => match profile.default_inventory.clone() {
                    Some(default) => Arc::new(InventoryBuilder::build(
                        &profile,
                        &config,
                        default,
                        &catalog,
                        &self.codecs,
                    )?),
                    None => return Err(discovery_error),
                },
                Err(_) => return Err(discovery_error),
            },
        };
        *runtime.inventory.write().await = inventory.clone();
        let executable = Arc::new(ExecutableProviderInstance {
            config: config.clone(),
            profile,
            inventory,
            generation,
            runtime: runtime.clone(),
        });
        {
            let mut runtimes = self.runtimes.lock().await;
            if runtimes
                .insert(config.provider_instance_name.clone(), runtime.clone())
                .is_some()
            {
                runtime.stop().await;
                return Err(ProviderError::DuplicateInstance(
                    config.provider_instance_name.clone(),
                ));
            }
        }
        self.publish_instance(executable.clone()).await;
        if config.auto_sync_models {
            *runtime.task.lock().await = Some(tokio::spawn(runtime.clone().run(stop_rx)));
        }
        Ok(executable)
    }

    pub(crate) async fn refresh(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let runtime = self
            .runtimes
            .lock()
            .await
            .get(provider_instance_name)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownInstance(provider_instance_name.into()))?;
        runtime
            .refresh_once(true, ProviderRefreshTrigger::Manual, true)
            .await
    }

    pub(crate) async fn build_inventory_candidate(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<ProviderInventoryCandidate> {
        let runtime = self.runtime(provider_instance_name).await?;
        let _refresh = runtime.refresh_lock.lock().await;
        runtime.build_candidate().await
    }

    pub(crate) async fn commit_inventory_candidate(
        &self,
        candidate: ProviderInventoryCandidate,
        trigger: ProviderRefreshTrigger,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let runtime = self.runtime(&candidate.provider_instance_name).await?;
        let attempt_at_ms = now_ms()?;
        match runtime.commit_candidate(&candidate, true).await {
            Ok(()) => {
                runtime
                    .record_success(attempt_at_ms, candidate.inventory.health)
                    .await?;
                runtime.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Committed {
                        changed: true,
                        inventory_revision: candidate.inventory.inventory_revision.clone(),
                        metadata_applied_seq: candidate.inventory.metadata_applied_seq,
                    },
                );
                Ok(candidate.inventory)
            }
            Err(error) => {
                if !matches!(error, ProviderError::StaleCandidate) {
                    runtime.record_failure(attempt_at_ms, &error).await;
                }
                runtime.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                Err(error)
            }
        }
    }

    pub(crate) async fn reconcile_inventory(
        &self,
        catalog: Arc<CatalogSnapshot>,
    ) -> Vec<ProviderRefreshEvent> {
        *self.catalog.write().await = catalog.clone();
        let runtimes: Vec<_> = self.runtimes.lock().await.values().cloned().collect();
        let mut results = Vec::with_capacity(runtimes.len());
        for runtime in runtimes {
            let refreshed = match runtime
                .discovery
                .refresh_catalog(&catalog, &runtime.profile.provider_profile_id)
                .await
            {
                Ok(()) => {
                    runtime
                        .refresh_once(true, ProviderRefreshTrigger::Reconciliation, true)
                        .await
                }
                Err(error) => Err(error),
            };
            let outcome = match refreshed {
                Ok(inventory) => ProviderRefreshOutcome::Committed {
                    changed: true,
                    inventory_revision: inventory.inventory_revision.clone(),
                    metadata_applied_seq: inventory.metadata_applied_seq,
                },
                Err(error) => ProviderRefreshOutcome::Failed {
                    kind: ProviderRefreshFailure::from_error(&error),
                },
            };
            results.push(ProviderRefreshEvent {
                provider_instance_name: runtime.config.provider_instance_name.clone(),
                trigger: ProviderRefreshTrigger::Reconciliation,
                outcome,
            });
        }
        results
    }

    pub(crate) async fn replace(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let name = config.provider_instance_name.clone();
        self.stop_and_remove_unlocked(&name).await?;
        self.start_unpublished(config, discovery).await
    }

    pub(crate) async fn stop_and_remove(&self, name: &str) -> ProviderResult<()> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.stop_and_remove_unlocked(name).await
    }

    async fn stop_and_remove_unlocked(&self, name: &str) -> ProviderResult<()> {
        let runtime = self.runtimes.lock().await.remove(name);
        let Some(runtime) = runtime else {
            return Ok(());
        };
        runtime.stop().await;
        let current = self.registry.read().await.clone();
        let mut next = current.instances.clone();
        next.remove(name);
        *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let runtimes = {
            let mut runtimes = self.runtimes.lock().await;
            std::mem::take(&mut *runtimes)
        };
        for runtime in runtimes.values() {
            runtime.stop().await;
        }
        *self.registry.write().await = Arc::new(ProviderRegistry::default());
    }

    async fn publish_instance(&self, instance: Arc<ExecutableProviderInstance>) {
        let current = self.registry.read().await.clone();
        let mut next = current.instances.clone();
        next.insert(instance.config.provider_instance_name.clone(), instance);
        *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
    }

    pub(super) async fn runtime(&self, name: &str) -> ProviderResult<Arc<ProviderRuntime>> {
        self.runtimes
            .lock()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownInstance(name.into()))
    }
}

async fn load_lkgs(
    runtime: &ProviderRuntime,
) -> ProviderResult<Option<Arc<ProviderInventorySnapshot>>> {
    let Some(record) = runtime
        .store
        .load(&runtime.config.provider_instance_name)
        .await?
    else {
        return Ok(None);
    };
    if record.provider_profile_id != runtime.profile.provider_profile_id
        || record.protocol_adapter_id != runtime.config.protocol_adapter_id
    {
        return Ok(None);
    }
    let inventory: ProviderInventorySnapshot = serde_json::from_value(record.snapshot.clone())
        .map_err(|error| ProviderError::Inventory(error.to_string()))?;
    validate_inventory_identity(&inventory, &runtime.profile, &runtime.config)?;
    if inventory.provider_model_list_fingerprint != record.provider_model_list_fingerprint
        || inventory.metadata_applied_seq != record.metadata_applied_seq
        || inventory.inventory_revision != record.inventory_revision
        || inventory.discovered_at_ms != record.discovered_at_ms
    {
        return Err(ProviderError::Inventory(
            "LKGS row columns do not match its inventory snapshot".into(),
        ));
    }
    let mut model_ids = BTreeSet::new();
    for model in &inventory.models {
        if !model_ids.insert(&model.provider_model_id) {
            return Err(ProviderError::Inventory(
                "LKGS contains duplicate provider model IDs".into(),
            ));
        }
        for api_type in &model.api_types {
            let name = api_type_name(*api_type)?;
            let operation = model.operations.get(&name).ok_or_else(|| {
                ProviderError::Inventory("LKGS model is missing an operation binding".into())
            })?;
            runtime
                .codecs
                .operation_descriptor(&inventory.protocol_adapter_id, operation, *api_type)
                .map_err(|error| ProviderError::Inventory(error.to_string()))?;
        }
    }
    Ok(Some(Arc::new(inventory)))
}

fn validate_inventory_identity(
    inventory: &ProviderInventorySnapshot,
    profile: &ProviderProfile,
    instance: &ProviderInstanceConfig,
) -> ProviderResult<()> {
    if inventory.schema_version != INVENTORY_SCHEMA_VERSION
        || inventory.provider_instance_name != instance.provider_instance_name
        || inventory.provider_profile_id != profile.provider_profile_id
        || inventory.protocol_adapter_id != instance.protocol_adapter_id
        || inventory.provider_model_list_fingerprint.trim().is_empty()
    {
        return Err(ProviderError::Inventory(
            "LKGS identity or schema does not match the provider instance".into(),
        ));
    }
    Ok(())
}

fn empty_inventory(
    profile: &ProviderProfile,
    instance: &ProviderInstanceConfig,
    metadata_applied_seq: u64,
) -> ProviderInventorySnapshot {
    ProviderInventorySnapshot {
        schema_version: INVENTORY_SCHEMA_VERSION,
        provider_instance_name: instance.provider_instance_name.clone(),
        provider_profile_id: profile.provider_profile_id.clone(),
        protocol_adapter_id: instance.protocol_adapter_id.clone(),
        provider_model_list_fingerprint: "pending".into(),
        metadata_applied_seq,
        inventory_revision: None,
        discovered_at_ms: 0,
        health: ProviderHealthState::Unknown,
        models: Vec::new(),
    }
}
