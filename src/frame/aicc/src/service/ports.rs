use super::*;

pub(crate) struct SystemConfigSettingsStore {
    service_url: String,
}

pub(crate) struct RuntimeAuthorizer;

#[async_trait]
impl ServiceAuthorizer for RuntimeAuthorizer {
    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        let token = context
            .token
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("session token is required".to_string()))?;
        let mut request = RPCRequest::new("aicc.authorize", Value::Null);
        request.token = Some(token.clone());
        let (user_id, target) = get_buckyos_api_runtime()?
            .enforce(&request, action, resource)
            .await?;
        Ok(AuthorizedCaller {
            tenant_id: user_id.clone(),
            user_id,
            app_id: Some(target),
            token,
        })
    }
}

impl SystemConfigSettingsStore {
    pub(crate) fn new(service_url: impl Into<String>) -> Self {
        Self {
            service_url: service_url.into(),
        }
    }

    fn client(&self, token: &str) -> SystemConfigClient {
        SystemConfigClient::new(Some(&self.service_url), Some(token))
    }
}

#[async_trait]
impl SettingsStore for SystemConfigSettingsStore {
    async fn load(&self, token: &str) -> Result<StoredSettings, RPCErrors> {
        let value = self
            .client(token)
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        Ok(StoredSettings {
            document: SettingsDocument::parse(value.version, &value.value).map_err(to_rpc_error)?,
        })
    }

    async fn compare_and_swap(
        &self,
        token: &str,
        expected_revision: u64,
        settings: &AiccSettings,
    ) -> Result<u64, RPCErrors> {
        let client = self.client(token);
        let serialized = serde_json::to_string(settings).map_err(to_rpc_error)?;
        let mut actions = HashMap::new();
        actions.insert(
            crate::settings::AICC_SETTINGS_KEY.to_string(),
            KVAction::Update(serialized.clone()),
        );
        client
            .exec_tx(
                actions,
                Some((
                    crate::settings::AICC_SETTINGS_KEY.to_string(),
                    expected_revision,
                )),
            )
            .await
            .map_err(|error| {
                conflict_error(expected_revision, expected_revision.saturating_add(1))
                    .or_reason(error)
            })?;
        let persisted = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        if persisted.value != serialized {
            return Err(RPCErrors::ReasonError(
                "settings changed after compare-and-swap".to_string(),
            ));
        }
        Ok(persisted.version)
    }
}

trait RpcErrorContext {
    fn or_reason(self, source: impl std::fmt::Display) -> RPCErrors;
}

impl RpcErrorContext for RPCErrors {
    fn or_reason(self, source: impl std::fmt::Display) -> RPCErrors {
        RPCErrors::ReasonError(format!("{self}: {source}"))
    }
}

pub(crate) struct StorageQueryPort {
    storage: Arc<AiccStorage>,
}

impl StorageQueryPort {
    pub(crate) fn new(storage: Arc<AiccStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl UsageQueryPort for StorageQueryPort {
    async fn query_usage(
        &self,
        request: QueryUsageRequest,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(to_rpc_error)?
            .as_millis() as i64;
        self.storage
            .query_usage(&request, now_ms)
            .await
            .map_err(to_rpc_error)
    }

    async fn query_trace(
        &self,
        tenant_id: &str,
        request: QueryRouteTraceRequest,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        self.storage
            .query_route_traces(tenant_id, &request)
            .await
            .map_err(to_rpc_error)
    }
}

pub(crate) struct CloudUpdateDriverMetadataPort {
    service_url: String,
    manager: Arc<CloudUpdateManager>,
    runtime: Arc<RuntimeState>,
}

impl CloudUpdateDriverMetadataPort {
    pub(crate) fn new(
        service_url: impl Into<String>,
        manager: Arc<CloudUpdateManager>,
        runtime: Arc<RuntimeState>,
    ) -> Self {
        Self {
            service_url: service_url.into(),
            manager,
            runtime,
        }
    }

    fn client(&self, token: &str) -> SystemConfigClient {
        SystemConfigClient::new(Some(&self.service_url), Some(token))
    }

    async fn view(&self) -> DriverMetadataUpdateView {
        let config = self.manager.config().await;
        let status = self.manager.status().await;
        let runtime = self.runtime.metadata_status().await;
        let providers = runtime
            .providers
            .iter()
            .map(|(name, state)| {
                buckyos_api::DriverMetadataProviderStatus::new(
                    name.clone(),
                    state.metadata_applied_seq,
                )
            })
            .collect::<Vec<_>>();
        let convergence_degraded = runtime.providers.values().any(|provider| {
            provider.last_error.is_some() || provider.metadata_applied_seq < runtime.target_seq
        });
        DriverMetadataUpdateView {
            enabled: config.enabled,
            source_url: config.source_url.clone(),
            source_configured: config.source_url.is_some(),
            interval_secs: config.interval_secs,
            metadata_target_seq: runtime.target_seq,
            providers,
            status: if !config.enabled {
                DriverMetadataUpdateStatus::Disabled
            } else if status.updating {
                DriverMetadataUpdateStatus::Updating
            } else if status.last_error.is_some() {
                DriverMetadataUpdateStatus::Error
            } else if convergence_degraded {
                DriverMetadataUpdateStatus::Degraded
            } else if status.last_success_at_ms.is_some() {
                DriverMetadataUpdateStatus::Healthy
            } else {
                DriverMetadataUpdateStatus::Idle
            },
            active_revision: status.active_revision,
            last_attempt_at_ms: status.last_attempt_at_ms,
            last_success_at_ms: status.last_success_at_ms,
            last_error: status.last_error,
            consecutive_failures: status.consecutive_failures,
        }
    }
}

#[async_trait]
impl DriverMetadataPort for CloudUpdateDriverMetadataPort {
    async fn get(&self) -> Result<DriverMetadataUpdateView, RPCErrors> {
        Ok(self.view().await)
    }

    async fn set(
        &self,
        token: &str,
        expected_settings_revision: u64,
        request: DriverMetadataUpdateSetReq,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        let current = self.manager.config().await;
        let next = CloudUpdateConfig {
            enabled: request.enabled,
            source_url: request.source_url.or(current.source_url),
            interval_secs: request.interval_secs.unwrap_or(current.interval_secs),
        };
        next.validate().map_err(to_rpc_error)?;
        let client = self.client(token);
        let settings = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        if settings.version != expected_settings_revision {
            return Err(conflict_error(expected_settings_revision, settings.version));
        }
        let action = match client.get(CLOUD_UPDATE_CONFIG_KEY).await {
            Ok(_) => KVAction::Update(serde_json::to_string(&next).map_err(to_rpc_error)?),
            Err(SystemConfigError::KeyNotFound(_)) => {
                KVAction::Create(serde_json::to_string(&next).map_err(to_rpc_error)?)
            }
            Err(error) => return Err(to_rpc_error(error)),
        };
        let mut actions = HashMap::new();
        actions.insert(CLOUD_UPDATE_CONFIG_KEY.to_string(), action);
        actions.insert(
            crate::settings::AICC_SETTINGS_KEY.to_string(),
            KVAction::Update(settings.value.clone()),
        );
        client
            .exec_tx(
                actions,
                Some((
                    crate::settings::AICC_SETTINGS_KEY.to_string(),
                    expected_settings_revision,
                )),
            )
            .await
            .map_err(to_rpc_error)?;
        let persisted_revision = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?
            .version;
        if persisted_revision != expected_settings_revision.saturating_add(1) {
            return Err(RPCErrors::ReasonError(
                "metadata update CAS returned an unexpected settings revision".to_string(),
            ));
        }
        self.manager.set_config(next).await.map_err(to_rpc_error)?;
        Ok(DriverMetadataUpdateSetResponse {
            ok: true,
            settings_revision: persisted_revision,
            settings: self.view().await,
            runtime_apply: DriverMetadataRuntimeApply {
                ok: true,
                refresh_scheduled: Some(request.enabled),
                error: None,
            },
        })
    }
}

pub(crate) struct RuntimeServiceAdapter {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
}

impl RuntimeServiceAdapter {
    pub(crate) fn new(runtime: Arc<RuntimeState>, codecs: Arc<CodecRegistry>) -> Self {
        Self { runtime, codecs }
    }

    async fn snapshot(&self) -> RuntimeAdminSnapshot {
        runtime_admin_snapshot(self.runtime.capture().await.as_ref(), self.codecs.as_ref())
    }
}

struct PreparedRuntimeAdapter {
    prepared: PreparedRuntimeMutation,
    codecs: Arc<CodecRegistry>,
}

#[async_trait]
impl PreparedSettingsRuntime for PreparedRuntimeAdapter {
    fn expected_revision(&self) -> u64 {
        self.prepared.expected_settings_revision()
    }

    fn settings_revision(&self) -> u64 {
        self.prepared.settings_revision()
    }

    async fn publish(self: Box<Self>) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        let Self { prepared, codecs } = *self;
        let snapshot = prepared.publish().await;
        Ok(runtime_admin_snapshot(snapshot.as_ref(), codecs.as_ref()))
    }

    async fn discard(self: Box<Self>) {
        self.prepared.discard().await;
    }
}

#[async_trait]
impl ServiceRuntime for RuntimeServiceAdapter {
    async fn capture(&self) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        Ok(self.snapshot().await)
    }

    async fn prepare_settings(
        &self,
        settings: SettingsDocument,
    ) -> Result<Box<dyn PreparedSettingsRuntime>, RPCErrors> {
        let prepared = self
            .runtime
            .prepare_reload(settings)
            .await
            .map_err(to_rpc_error)?;
        Ok(Box::new(PreparedRuntimeAdapter {
            prepared,
            codecs: self.codecs.clone(),
        }))
    }

    async fn refresh_provider(
        &self,
        provider_instance_name: &str,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        let snapshot = self
            .runtime
            .refresh_provider(provider_instance_name)
            .await
            .map_err(to_rpc_error)?;
        Ok(runtime_admin_snapshot(
            snapshot.as_ref(),
            self.codecs.as_ref(),
        ))
    }
}

pub(crate) struct TaskManagerExecutionPort;

impl TaskManagerExecutionPort {
    pub(crate) fn new() -> Self {
        Self
    }

    async fn client(&self, operation: &str) -> Result<TaskManagerClient, buckyos_api::AiccError> {
        get_buckyos_api_runtime()
            .map_err(|err| task_manager_error(format!("{operation}.runtime"), err))?
            .get_task_mgr_client()
            .await
            .map_err(|err| task_manager_error(format!("{operation}.get_task_mgr_client"), err))
    }
}

#[async_trait]
impl TaskManagerPort for TaskManagerExecutionPort {
    async fn ensure_task(&self, spec: TaskSpec) -> Result<TaskBinding, buckyos_api::AiccError> {
        let operation = format!(
            "create_delegated_task method={} trace_id={} idempotency_key={}",
            spec.method,
            spec.trace_id.as_deref().unwrap_or("<none>"),
            spec.idempotency_key
        );
        let task = self
            .client(&operation)
            .await?
            .create_delegated_task(CreateDelegatedTaskReq {
                task_id: None,
                name: format!("AICC {}", spec.method),
                schema_id: AICC_COMPUTE_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: json!({
                    "request": {
                        "version": 1,
                        "tenant_id": spec.tenant_id,
                        "trace_id": spec.trace_id,
                        "request": spec.input,
                    }
                }),
                creator: ActorRef::new(
                    spec.user_id,
                    spec.caller_app_id.ok_or_else(|| {
                        task_manager_error(
                            operation.clone(),
                            RPCErrors::ReasonError(
                                "AICC task caller app identity is unavailable".to_string(),
                            ),
                        )
                    })?,
                ),
                runner_app_instance_id: None,
                parent_id: spec.parent_id,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: None,
                idempotency_key: spec.idempotency_key,
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| task_manager_error(operation, err))?;
        Ok(TaskBinding {
            event_ref: buckyos_api::task_mgr_task_event_path(&task.task_id),
            task_id: task.task_id,
        })
    }

    async fn report_state(
        &self,
        task_id: &str,
        state: ExecutionState,
        data: Value,
    ) -> Result<(), buckyos_api::AiccError> {
        if matches!(state, ExecutionState::Running) {
            let operation = format!("runner_start task_id={task_id}");
            self.client(&operation)
                .await?
                .runner_start(task_id)
                .await
                .map_err(|err| task_manager_error(operation, err))?;
        }
        let operation = format!("runner_progress task_id={task_id} state={state:?}");
        self.client(&operation)
            .await?
            .runner_progress(task_id, Some(data), None)
            .await
            .map(|_| ())
            .map_err(|err| task_manager_error(operation, err))
    }

    async fn commit_result(
        &self,
        task_id: &str,
        output: &ExecutionOutput,
    ) -> Result<(), buckyos_api::AiccError> {
        let operation = format!("runner_complete task_id={task_id}");
        self.client(&operation)
            .await?
            .runner_complete(
                task_id,
                json!({
                    "result": {
                        "output": serde_json::to_value(output).map_err(|_| {
                            buckyos_api::AiccError::new(
                                buckyos_api::AiccErrorCode::InternalError,
                                "execution result could not be serialized",
                            )
                        })?
                    }
                }),
            )
            .await
            .map(|_| ())
            .map_err(|err| task_manager_error(operation, err))
    }

    async fn fail_task(
        &self,
        task_id: &str,
        error: &buckyos_api::AiccError,
    ) -> Result<(), buckyos_api::AiccError> {
        let operation = format!(
            "runner_fail task_id={} error_code={}",
            task_id,
            error.code.as_str()
        );
        self.client(&operation)
            .await?
            .runner_fail(
                task_id,
                error.code.as_str(),
                error.message.clone(),
                Some(error.to_task_event_data()),
            )
            .await
            .map(|_| ())
            .map_err(|err| task_manager_error(operation, err))
    }

    async fn cancel_task(
        &self,
        task_id: &str,
        user_id: &str,
        caller_app_id: &str,
    ) -> Result<(), buckyos_api::AiccError> {
        let operation = format!(
            "request_delegated_control action=cancel task_id={task_id} user_id={user_id} caller_app_id={caller_app_id}"
        );
        let client = self.client(&operation).await?;
        let request_id = format!("aicc-cancel-{}", next_inference_id());
        let requested = client
            .request_delegated_control(RequestDelegatedControlReq {
                controller: ActorRef::new(user_id, caller_app_id),
                task_id: task_id.to_string(),
                action: TaskControlAction::Cancel,
                request_id: request_id.clone(),
                expected_revision: None,
            })
            .await
            .map_err(|err| task_manager_error(operation.clone(), err))?;
        let task = match requested {
            RequestControlResult::Task { task } => task,
            RequestControlResult::Batch { .. } => {
                return Err(task_manager_error(
                    operation,
                    RPCErrors::ReasonError(
                        "TaskMgr returned a batch result for a single task cancellation"
                            .to_string(),
                    ),
                ));
            }
        };
        let app_instance_id = match &task.executor {
            TaskExecutor::App {
                app_instance_id, ..
            } => app_instance_id.clone(),
            _ => None,
        };
        let ack_operation = format!(
            "ack_control action=cancel task_id={} request_id={request_id}",
            task.task_id
        );
        client
            .ack_control(AckControlReq {
                envelope: RunnerWriteEnvelope {
                    task_id: task.task_id,
                    app_instance_id,
                    runner_epoch: task.runner_epoch,
                    expected_revision: task.revision,
                },
                request_id,
                applied: true,
                reject_reason: None,
            })
            .await
            .map(|_| ())
            .map_err(|err| task_manager_error(ack_operation, err))
    }
}
