use super::*;

pub(crate) struct RuntimeProviderExecutionPort {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
    resource_store: Arc<dyn ResourceStore>,
    url_fetcher: Arc<dyn UrlResourceFetcher>,
    storage: Arc<AiccStorage>,
    model_health: Arc<ModelHealthRegistry>,
}

impl RuntimeProviderExecutionPort {
    pub(crate) fn new(
        runtime: Arc<RuntimeState>,
        codecs: Arc<CodecRegistry>,
        resource_store: Arc<dyn ResourceStore>,
        url_fetcher: Arc<dyn UrlResourceFetcher>,
        storage: Arc<AiccStorage>,
        model_health: Arc<ModelHealthRegistry>,
    ) -> Self {
        Self {
            runtime,
            codecs,
            resource_store,
            url_fetcher,
            storage,
            model_health,
        }
    }

    async fn remember_artifact_url_sources(
        storage: &AiccStorage,
        provider_instance_name: &str,
        protocol_adapter_id: &str,
        context: &ResourceAccessContext,
        output: &ProtocolOutput,
    ) -> Result<(), ProtocolError> {
        for artifact in &output.artifacts {
            let buckyos_api::ResourceRef::Url { url, .. } = &artifact.resource else {
                continue;
            };
            storage
                .remember_artifact_url_source(&ArtifactUrlSourceRecord {
                    url: url.clone(),
                    provider_instance_name: provider_instance_name.to_owned(),
                    protocol_adapter_id: protocol_adapter_id.to_owned(),
                    artifact_id: artifact
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("artifact_id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| Some(artifact.name.clone())),
                    tenant_id: context.tenant_id.clone(),
                    user_id: context.caller_id.clone(),
                    caller_app_id: None,
                    request_id: context.request_id.clone(),
                    created_at_ms: now_ms() as i64,
                })
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("artifact URL source registration failed")
                })?;
        }
        Ok(())
    }

    async fn remember_call_artifact_urls(
        &self,
        call: &ResolvedProviderCall,
        output: &ProtocolOutput,
    ) -> Result<(), ProtocolError> {
        let has_url = output
            .artifacts
            .iter()
            .any(|artifact| matches!(artifact.resource, buckyos_api::ResourceRef::Url { .. }));
        if !has_url {
            return Ok(());
        }
        let context = call.resource_access_context.as_ref().ok_or_else(|| {
            ProtocolError::invalid_configuration("artifact URL context is missing")
        })?;
        Self::remember_artifact_url_sources(
            self.storage.as_ref(),
            &call.provider_instance_name,
            &call.protocol_adapter_id,
            context,
            output,
        )
        .await
    }

    pub(crate) async fn open_artifact_url_reader(
        &self,
        tenant_id: &str,
        url: &str,
        artifact_id: Option<&str>,
    ) -> Result<crate::protocol::ArtifactUrlReader, AiccError> {
        let source = self
            .storage
            .artifact_url_source(url)
            .await
            .map_err(|_| {
                AiccError::new(
                    AiccErrorCode::InternalError,
                    "artifact URL source lookup failed",
                )
            })?
            .ok_or_else(|| {
                AiccError::new(
                    AiccErrorCode::ResourceInvalid,
                    "URL is not a registered Provider artifact",
                )
            })?;
        if source.tenant_id != tenant_id {
            return Err(AiccError::new(
                AiccErrorCode::PolicyDenied,
                "Provider artifact belongs to another tenant",
            ));
        }
        if artifact_id.is_some_and(|artifact_id| source.artifact_id.as_deref() != Some(artifact_id))
        {
            return Err(AiccError::new(
                AiccErrorCode::ResourceInvalid,
                "artifact ID does not match the registered Provider artifact",
            ));
        }
        let snapshot = self.runtime.capture().await;
        let provider = snapshot
            .providers
            .get(&source.provider_instance_name)
            .ok_or_else(|| {
                AiccError::new(
                    AiccErrorCode::NoProviderAvailable,
                    "artifact ProviderInstance is unavailable",
                )
            })?;
        if provider.config.protocol_adapter_id != source.protocol_adapter_id {
            return Err(AiccError::new(
                AiccErrorCode::ProviderError,
                "artifact ProviderInstance Adapter has changed",
            ));
        }
        provider
            .open_artifact_url_reader(self.codecs.as_ref(), url)
            .await
            .map_err(|error| AiccError {
                code: AiccErrorCode::ProviderError,
                message: error.message,
                provider_code: error.provider_code,
                retriable: matches!(
                    error.kind,
                    ProtocolErrorKind::Timeout | ProtocolErrorKind::Transport
                ),
                details: None,
            })
    }

    async fn materialize_embedding_output(
        &self,
        call: &ResolvedProviderCall,
        mut output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        let AiccCall::EmbeddingText(request) = &call.input.canonical_request else {
            return Ok(output);
        };
        let data = output
            .value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| ProtocolError::invalid_response("embedding output is missing data"))?;
        let encoded = serde_json::to_vec(&output.value).map_err(|_| {
            ProtocolError::invalid_configuration("embedding output could not be serialized")
        })?;
        let materialize = match request.prefer_artifact.as_ref() {
            Some(Value::Bool(value)) => *value,
            Some(Value::String(value)) if value == "auto" => {
                request.items.len() > 100 || encoded.len() > 1024 * 1024
            }
            None => request.items.len() > 100 || encoded.len() > 1024 * 1024,
            Some(_) => {
                return Err(ProtocolError::invalid_request(
                    "prefer_artifact must be true, false, or auto",
                ));
            }
        };
        if !materialize {
            return Ok(output);
        }
        let first = data.first().ok_or_else(|| {
            ProtocolError::invalid_response("embedding output must contain at least one row")
        })?;
        let dimensions = first
            .get("embedding")
            .and_then(Value::as_array)
            .map(Vec::len)
            .filter(|value| *value > 0)
            .ok_or_else(|| ProtocolError::invalid_response("embedding dimensions are invalid"))?;
        let space = first
            .get("embedding_space_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProtocolError::invalid_response("embedding space is missing"))?;
        if data.iter().any(|row| {
            row.get("embedding").and_then(Value::as_array).map(Vec::len) != Some(dimensions)
                || row.get("embedding_space_id").and_then(Value::as_str) != Some(space)
        }) {
            return Err(ProtocolError::invalid_response(
                "embedding rows do not share dimensions and space",
            ));
        }
        let context = call.resource_access_context.as_ref().ok_or_else(|| {
            ProtocolError::invalid_configuration("embedding artifact context is missing")
        })?;
        let manager = ResourceManager::new(
            Arc::new(AuthenticatedResourceAuthorizer {
                tenant_id: context.tenant_id.clone(),
                caller_id: context.caller_id.clone(),
                storage: self.storage.clone(),
            }),
            self.resource_store.clone(),
            self.url_fetcher.clone(),
            ResourceLimits::default(),
        )
        .map_err(|_| ProtocolError::invalid_configuration("artifact writer is unavailable"))?;
        let artifact = manager
            .write_artifact(
                context,
                &encoded,
                ArtifactSpec {
                    name: format!("embedding-{}.json", context.request_id),
                    mime: "application/json".to_string(),
                    attributes: Map::new(),
                    embedding: Some(EmbeddingArtifactMetadata {
                        rows: data.len() as u64,
                        dimensions: dimensions as u64,
                        space: space.to_string(),
                    }),
                },
            )
            .await
            .map_err(|_| ProtocolError::invalid_configuration("embedding artifact write failed"))?;
        let buckyos_api::ResourceRef::NamedObject { obj_id } = &artifact.resource else {
            return Err(ProtocolError::invalid_configuration(
                "embedding artifact did not produce a Named Object",
            ));
        };
        self.storage
            .remember_artifact_scope(
                &obj_id.to_string(),
                &context.tenant_id,
                &context.caller_id,
                None,
                now_ms() as i64,
            )
            .await
            .map_err(|_| {
                ProtocolError::invalid_configuration("embedding artifact ownership write failed")
            })?;
        let resource = artifact.resource.clone();
        output.value = json!({"data": [], "data_resource": resource});
        output.artifacts.push(artifact);
        Ok(output)
    }

    /// Whether `output` actually carries inline payloads that have to be
    /// relocated into the artifact store. The overwhelming majority of calls
    /// (every text / chat / tool response) carry none, and for those this
    /// materializer is a pure no-op — so it must not demand artifact scope.
    fn has_inline_artifacts(output: &ProtocolOutput) -> bool {
        let mut value_resources = Vec::new();
        collect_inline_base64_resource_refs(&output.value, &mut value_resources);
        if !value_resources.is_empty() {
            return true;
        }
        output
            .artifacts
            .iter()
            .any(|artifact| matches!(artifact.resource, buckyos_api::ResourceRef::Base64 { .. }))
    }

    async fn materialize_inline_artifact_output(
        &self,
        call: &ResolvedProviderCall,
        output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        if !Self::has_inline_artifacts(&output) {
            return Ok(output);
        }
        let context = call.resource_access_context.as_ref().ok_or_else(|| {
            ProtocolError::invalid_configuration("inline artifact context is missing")
        })?;
        self.materialize_inline_artifact_output_with_context(context, output)
            .await
    }

    async fn materialize_inline_artifact_output_with_context(
        &self,
        context: &ResourceAccessContext,
        mut output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        if !Self::has_inline_artifacts(&output) {
            return Ok(output);
        }
        let mut value_resources = Vec::new();
        collect_inline_base64_resource_refs(&output.value, &mut value_resources);
        let manager = ResourceManager::new(
            Arc::new(AuthenticatedResourceAuthorizer {
                tenant_id: context.tenant_id.clone(),
                caller_id: context.caller_id.clone(),
                storage: self.storage.clone(),
            }),
            self.resource_store.clone(),
            self.url_fetcher.clone(),
            ResourceLimits::default(),
        )
        .map_err(|_| ProtocolError::invalid_configuration("artifact writer is unavailable"))?;

        for index in 0..output.artifacts.len() {
            let (old_resource, name, mime, bytes) = match &output.artifacts[index].resource {
                buckyos_api::ResourceRef::Base64 { mime, data_base64 } => {
                    let bytes = BASE64_STANDARD.decode(data_base64).map_err(|_| {
                        ProtocolError::invalid_response("inline artifact is not valid base64")
                    })?;
                    (
                        output.artifacts[index].resource.clone(),
                        output.artifacts[index].name.clone(),
                        mime.clone(),
                        bytes,
                    )
                }
                _ => continue,
            };
            let mut artifact = manager
                .write_artifact(
                    context,
                    &bytes,
                    ArtifactSpec {
                        name,
                        mime: mime.clone(),
                        attributes: Map::new(),
                        embedding: None,
                    },
                )
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("inline artifact write failed")
                })?;
            let buckyos_api::ResourceRef::NamedObject { obj_id } = &artifact.resource else {
                return Err(ProtocolError::invalid_configuration(
                    "inline artifact did not produce a Named Object",
                ));
            };
            self.storage
                .remember_artifact_scope(
                    &obj_id.to_string(),
                    &context.tenant_id,
                    &context.caller_id,
                    None,
                    now_ms() as i64,
                )
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("inline artifact ownership write failed")
                })?;
            replace_resource_ref_value(&mut output.value, &old_resource, &artifact.resource)?;
            if artifact.mime.is_none() {
                artifact.mime = Some(mime);
            }
            append_materialized_artifact_metadata(
                &mut output.value,
                obj_id.to_string(),
                artifact.name.clone(),
                artifact.mime.clone(),
            );
            output.artifacts[index] = artifact;
        }
        for resource in value_resources {
            let buckyos_api::ResourceRef::Base64 { mime, data_base64 } = &resource else {
                continue;
            };
            let bytes = BASE64_STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_response("inline artifact is not valid base64")
            })?;
            let artifact = manager
                .write_artifact(
                    context,
                    &bytes,
                    ArtifactSpec {
                        name: format!("artifact-{}", output.artifacts.len() + 1),
                        mime: mime.clone(),
                        attributes: Map::new(),
                        embedding: None,
                    },
                )
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("inline artifact write failed")
                })?;
            let buckyos_api::ResourceRef::NamedObject { obj_id } = &artifact.resource else {
                return Err(ProtocolError::invalid_configuration(
                    "inline artifact did not produce a Named Object",
                ));
            };
            self.storage
                .remember_artifact_scope(
                    &obj_id.to_string(),
                    &context.tenant_id,
                    &context.caller_id,
                    None,
                    now_ms() as i64,
                )
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("inline artifact ownership write failed")
                })?;
            replace_resource_ref_value(&mut output.value, &resource, &artifact.resource)?;
            append_materialized_artifact_metadata(
                &mut output.value,
                obj_id.to_string(),
                artifact.name.clone(),
                artifact.mime.clone(),
            );
            output.artifacts.push(artifact);
        }
        Ok(output)
    }

    fn map_rerank_output(
        call: &ResolvedProviderCall,
        mut output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        let AiccCall::Rerank(request) = &call.input.canonical_request else {
            return Ok(output);
        };
        let results = output
            .value
            .get_mut("results")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| ProtocolError::invalid_response("rerank output is missing results"))?;
        for result in results {
            let index = result
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| ProtocolError::invalid_response("rerank result index is invalid"))?;
            let source = request.documents.get(index).ok_or_else(|| {
                ProtocolError::invalid_response("rerank result index is out of range")
            })?;
            let object = result.as_object_mut().ok_or_else(|| {
                ProtocolError::invalid_response("rerank result must be an object")
            })?;
            object.insert("id".to_owned(), Value::String(source.id.clone()));
            if request.return_documents.unwrap_or(false) {
                object.insert(
                    "document".to_owned(),
                    serde_json::to_value(source).map_err(|_| {
                        ProtocolError::invalid_configuration(
                            "rerank source document could not be serialized",
                        )
                    })?,
                );
            } else {
                object.remove("document");
            }
        }
        Ok(output)
    }

    fn validate_computer_output(
        call: &ResolvedProviderCall,
        output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        let AiccCall::ComputerUse(request) = &call.input.canonical_request else {
            return Ok(output);
        };
        let actions = output
            .value
            .get("actions")
            .and_then(Value::as_array)
            .ok_or_else(|| ProtocolError::invalid_response("computer-use output has no actions"))?;
        for action in actions {
            let kind = action.get("type").and_then(Value::as_str).ok_or_else(|| {
                ProtocolError::invalid_response("computer-use action has no type")
            })?;
            if !request
                .allowed_actions
                .iter()
                .any(|allowed| allowed == kind)
            {
                return Err(ProtocolError::invalid_response(
                    "Provider returned a computer action that the caller did not allow",
                ));
            }
        }
        Ok(output)
    }

    fn transport(limits: &CodecLimits) -> Result<HttpTransport, ProtocolError> {
        HttpTransport::new(HttpTransportConfig {
            request_timeout: limits.request_timeout,
            max_request_bytes: limits.max_request_bytes,
            max_response_bytes: limits.max_response_bytes,
            max_json_bytes: limits.max_response_bytes,
            ..HttpTransportConfig::default()
        })
    }

    pub(super) fn execution_mode(
        requested: ExecutionMode,
        supported: &BTreeSet<ExecutionMode>,
    ) -> Result<ExecutionMode, ProtocolError> {
        supported
            .contains(&requested)
            .then_some(requested)
            .ok_or_else(|| {
                ProtocolError::invalid_configuration(
                    "resolved execution mode is not supported by the selected binding",
                )
            })
    }

    async fn send_cancelable<T>(
        cancellation: &crate::protocol::Cancellation,
        future: impl std::future::Future<Output = Result<T, ProtocolError>>,
    ) -> Result<T, ProtocolError> {
        tokio::select! {
            result = future => result,
            _ = cancellation.cancelled() => Err(ProtocolError::new(
                ProtocolErrorKind::Cancelled,
                "Provider request was cancelled",
            )),
        }
    }

    async fn resume_context(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<(CodecContext, BTreeMap<String, Value>), NativeTaskResumeError> {
        let resume = binding
            .resume
            .as_ref()
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        let snapshot = self.runtime.capture().await;
        let provider = snapshot
            .providers
            .get(&binding.provider_instance_name)
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        let credential = match &resume.credential {
            Some(expected) => {
                let reference = &provider.config.credential.reference;
                if reference != &expected.reference
                    || credential_fingerprint(reference) != expected.fingerprint
                {
                    return Err(NativeTaskResumeError::CredentialUnavailable);
                }
                let resolved = provider
                    .resolve_credential()
                    .await
                    .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
                if resume_credential_kind(resolved.audit().kind) != expected.kind {
                    return Err(NativeTaskResumeError::CredentialUnavailable);
                }
                Some(resolved)
            }
            None => None,
        };
        let request_timeout = Duration::from_millis(resume.request_timeout_ms);
        let max_request_bytes = usize::try_from(resume.max_request_bytes)
            .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
        let max_response_bytes = usize::try_from(resume.max_response_bytes)
            .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
        let model = provider
            .inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == binding.provider_model_id)
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        Ok((
            CodecContext {
                base_url: resume.base_url.clone(),
                state_coordinate: buckyos_api::ProviderStateCoordinate {
                    normalized_base_url: crate::protocol::normalize_provider_base_url(
                        &resume.base_url,
                    )
                    .map_err(NativeTaskResumeError::Protocol)?,
                    adapter_type: binding.protocol_adapter_id.clone(),
                    origin_provider: model.model_driver_id.clone(),
                    origin_model: model.origin_model_id.clone(),
                },
                credential,
                resources: BTreeMap::new(),
                limits: CodecLimits {
                    request_timeout,
                    max_request_bytes,
                    max_response_bytes,
                },
            },
            resume.resolved_parameters.clone(),
        ))
    }

    async fn native_request(
        &self,
        binding: &PinnedProviderTask,
        operation: NativeTaskOperation,
        cancellation: Option<&crate::protocol::Cancellation>,
    ) -> Result<NativeTaskOutput, NativeTaskResumeError> {
        let remote_task_id = binding.remote_task_id.as_deref().ok_or_else(|| {
            NativeTaskResumeError::Protocol(ProtocolError::invalid_request(
                "native task binding has no remote task ID",
            ))
        })?;
        let (context, parameters) = self.resume_context(binding).await?;
        let request = self
            .codecs
            .encode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                &NativeTaskInput {
                    operation,
                    remote_task_id: Some(remote_task_id),
                    codec_input: None,
                    resolved_parameters: &parameters,
                    context: &context,
                },
            )
            .map_err(NativeTaskResumeError::Protocol)?;
        let transport =
            Self::transport(&context.limits).map_err(NativeTaskResumeError::Protocol)?;
        let response = match cancellation {
            Some(cancellation) => {
                Self::send_cancelable(cancellation, transport.send(request)).await
            }
            None => transport.send(request).await,
        }
        .map_err(NativeTaskResumeError::Protocol)?;
        let mut output = self
            .codecs
            .decode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                operation,
                response,
            )
            .await
            .map_err(NativeTaskResumeError::Protocol)?;
        if let NativeTaskOutput::Result(result) = &mut output {
            crate::protocol::bind_provider_state_source(
                &mut result.value,
                &context.state_coordinate,
            );
        }
        Ok(output)
    }
}

#[async_trait]
impl ProviderExecutionPort for RuntimeProviderExecutionPort {
    async fn start(
        &self,
        _runtime_generation: u64,
        call: &crate::call::ResolvedProviderCall,
        cancellation: crate::protocol::Cancellation,
    ) -> Result<ProviderExecution, ProviderStartFailure> {
        let started = Instant::now();
        let result = async {
            let descriptor = self
                .codecs
                .operation_descriptor(&call.protocol_adapter_id, &call.operation, call.api_type)
                .and_then(|operation| {
                    operation
                        .binding(call.api_type)
                        .map(|binding| (operation.supports_cancel, binding.execution_modes.clone()))
                })
                .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
            let execution_mode = Self::execution_mode(call.execution_mode, &descriptor.1)
                .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
            let transport = Self::transport(&call.context.limits)
                .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
            match execution_mode {
                ExecutionMode::Immediate => {
                    let request = self
                        .codecs
                        .encode(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            &call.input,
                            &call.context,
                        )
                        .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
                    let response = Self::send_cancelable(&cancellation, transport.send(request))
                        .await
                        .map_err(ProviderStartFailure::after_accept)?;
                    let decoded = self
                        .codecs
                        .decode(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            response,
                        )
                        .await
                        .map_err(ProviderStartFailure::after_accept)?;
                    match decoded {
                        crate::protocol::ProtocolExecution::Immediate(output) => {
                            let mut output = output;
                            crate::protocol::bind_provider_state_source(
                                &mut output.value,
                                &call.context.state_coordinate,
                            );
                            let output = self
                                .materialize_embedding_output(call, output)
                                .await
                                .map_err(ProviderStartFailure::after_accept)?;
                            let output = self
                                .materialize_inline_artifact_output(call, output)
                                .await
                                .map_err(ProviderStartFailure::after_accept)?;
                            let output = Self::map_rerank_output(call, output)
                                .map_err(ProviderStartFailure::after_accept)?;
                            let output = Self::validate_computer_output(call, output)
                                .map_err(ProviderStartFailure::after_accept)?;
                            self.remember_call_artifact_urls(call, &output)
                                .await
                                .map_err(ProviderStartFailure::after_accept)?;
                            Ok(ProviderExecution::Immediate(output))
                        }
                        _ => Err(ProviderStartFailure::after_accept(
                            ProtocolError::invalid_response(
                                "buffered Provider response returned an unexpected execution mode",
                            ),
                        )),
                    }
                }
                ExecutionMode::Stream => {
                    let request = self
                        .codecs
                        .encode(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            &call.input,
                            &call.context,
                        )
                        .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
                    let response =
                        Self::send_cancelable(&cancellation, transport.send_streaming(request))
                            .await
                            .map_err(ProviderStartFailure::after_accept)?;
                    let stream = self
                        .codecs
                        .decode_stream(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            response,
                        )
                        .await
                        .map_err(ProviderStartFailure::after_accept)?;
                    let source = call.context.state_coordinate.clone();
                    let storage = self.storage.clone();
                    let provider_instance_name = call.provider_instance_name.clone();
                    let protocol_adapter_id = call.protocol_adapter_id.clone();
                    let resource_context = call.resource_access_context.clone();
                    let events = stream.events.then(move |event| {
                        let source = source.clone();
                        let storage = storage.clone();
                        let provider_instance_name = provider_instance_name.clone();
                        let protocol_adapter_id = protocol_adapter_id.clone();
                        let resource_context = resource_context.clone();
                        async move {
                            let mut event = event?;
                            match &mut event {
                                ProtocolEvent::Delta(value) | ProtocolEvent::Progress(value) => {
                                    crate::protocol::bind_provider_state_source(value, &source);
                                }
                                ProtocolEvent::Final(output) => {
                                    crate::protocol::bind_provider_state_source(
                                        &mut output.value,
                                        &source,
                                    );
                                }
                            }
                            if let ProtocolEvent::Final(output) = &event {
                                if output.artifacts.iter().any(|artifact| {
                                    matches!(
                                        artifact.resource,
                                        buckyos_api::ResourceRef::Url { .. }
                                    )
                                }) {
                                    let context = resource_context.as_ref().ok_or_else(|| {
                                        ProtocolError::invalid_configuration(
                                            "artifact URL context is missing",
                                        )
                                    })?;
                                    Self::remember_artifact_url_sources(
                                        storage.as_ref(),
                                        &provider_instance_name,
                                        &protocol_adapter_id,
                                        context,
                                        output,
                                    )
                                    .await?;
                                }
                            }
                            Ok(event)
                        }
                    });
                    Ok(ProviderExecution::Stream(ProtocolStream {
                        events: Box::pin(events),
                    }))
                }
                ExecutionMode::NativeTask => {
                    let request = self
                        .codecs
                        .encode_native(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            &NativeTaskInput {
                                operation: NativeTaskOperation::Submit,
                                remote_task_id: None,
                                codec_input: Some(&call.input),
                                resolved_parameters: &call.input.resolved_parameters,
                                context: &call.context,
                            },
                        )
                        .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
                    let response = Self::send_cancelable(&cancellation, transport.send(request))
                        .await
                        .map_err(ProviderStartFailure::after_accept)?;
                    let output = self
                        .codecs
                        .decode_native(
                            &call.protocol_adapter_id,
                            &call.operation,
                            call.api_type,
                            NativeTaskOperation::Submit,
                            response,
                        )
                        .await
                        .map_err(ProviderStartFailure::after_accept)?;
                    let NativeTaskOutput::Submitted(handle) = output else {
                        return Err(ProviderStartFailure::after_accept(
                            ProtocolError::invalid_response(
                                "native submit returned a non-submit result",
                            ),
                        ));
                    };
                    let credential =
                        call.context
                            .credential
                            .as_ref()
                            .map(|credential| ResumeCredential {
                                reference: call.credential_reference.clone(),
                                kind: resume_credential_kind(credential.audit().kind),
                                header_name: call.credential_header_name.clone(),
                                fingerprint: credential_fingerprint(&call.credential_reference),
                            });
                    Ok(ProviderExecution::NativeTask {
                        handle,
                        resume: NativeTaskResumeDescriptor {
                            base_url: call.context.base_url.clone(),
                            credential,
                            resource_access_context: call
                                .resource_access_context
                                .clone()
                                .ok_or_else(|| {
                                    ProviderStartFailure::after_accept(
                                        ProtocolError::invalid_configuration(
                                            "native task resource context is missing",
                                        ),
                                    )
                                })?,
                            resolved_parameters: call.input.resolved_parameters.clone(),
                            request_timeout_ms: call.context.limits.request_timeout.as_millis()
                                as u64,
                            max_request_bytes: call.context.limits.max_request_bytes as u64,
                            max_response_bytes: call.context.limits.max_response_bytes as u64,
                        },
                    })
                }
            }
        }
        .await;
        let result = match result {
            Ok(ProviderExecution::Stream(stream)) => {
                Ok(ProviderExecution::Stream(observed_protocol_stream(
                    stream,
                    self.model_health.clone(),
                    call.exact_model.clone(),
                    call.provider_instance_name.clone(),
                    started,
                )))
            }
            result => result,
        };
        let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
        match &result {
            Ok(ProviderExecution::Stream(_)) => {}
            Ok(_) => self.model_health.record_success(
                &call.exact_model,
                &call.provider_instance_name,
                latency_ms,
                now_ms(),
            ),
            Err(failure) => {
                let kind = health_failure_kind(&failure.error);
                self.model_health.record_failure(
                    &call.exact_model,
                    &call.provider_instance_name,
                    latency_ms,
                    kind,
                    now_ms(),
                );
            }
        }
        result
    }

    async fn poll_native(
        &self,
        binding: &PinnedProviderTask,
        cancellation: crate::protocol::Cancellation,
    ) -> Result<NativeTaskPoll, NativeTaskResumeError> {
        return match self
            .native_request(binding, NativeTaskOperation::Status, Some(&cancellation))
            .await?
        {
            NativeTaskOutput::Status {
                state, result_ref, ..
            } if state == crate::protocol::NativeTaskState::Succeeded => {
                let mut result_binding = binding.clone();
                if let Some(result_ref) = result_ref {
                    result_binding.remote_task_id = Some(result_ref);
                }
                match self
                    .native_request(
                        &result_binding,
                        NativeTaskOperation::Result,
                        Some(&cancellation),
                    )
                    .await?
                {
                    NativeTaskOutput::Result(output) => {
                        let output = self
                            .materialize_inline_artifact_output_with_context(
                                &binding
                                    .resume
                                    .as_ref()
                                    .ok_or(NativeTaskResumeError::CredentialUnavailable)?
                                    .resource_access_context,
                                output,
                            )
                            .await
                            .map_err(NativeTaskResumeError::Protocol)?;
                        Self::remember_artifact_url_sources(
                            self.storage.as_ref(),
                            &binding.provider_instance_name,
                            &binding.protocol_adapter_id,
                            &binding
                                .resume
                                .as_ref()
                                .ok_or(NativeTaskResumeError::CredentialUnavailable)?
                                .resource_access_context,
                            &output,
                        )
                        .await
                        .map_err(NativeTaskResumeError::Protocol)?;
                        Ok(NativeTaskPoll::Complete(output))
                    }
                    _ => Err(NativeTaskResumeError::Protocol(
                        ProtocolError::invalid_response(
                            "native result returned an unexpected response",
                        ),
                    )),
                }
            }
            NativeTaskOutput::Status {
                state:
                    state @ (crate::protocol::NativeTaskState::Submitted
                    | crate::protocol::NativeTaskState::Queued
                    | crate::protocol::NativeTaskState::Running),
                retry_after,
                ..
            } => Ok(NativeTaskPoll::Pending(state, None, retry_after)),
            NativeTaskOutput::Status {
                state: crate::protocol::NativeTaskState::Cancelled,
                ..
            } => Ok(NativeTaskPoll::Failed(ProtocolError::new(
                ProtocolErrorKind::Cancelled,
                "native Provider task was cancelled",
            ))),
            NativeTaskOutput::Status {
                state: crate::protocol::NativeTaskState::Failed,
                ..
            } => Ok(NativeTaskPoll::Failed(ProtocolError::invalid_response(
                "native Provider task reported failure",
            ))),
            _ => Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_response("native status returned an unexpected result"),
            )),
        };
    }

    async fn cancel_native(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<bool, NativeTaskResumeError> {
        match self
            .native_request(binding, NativeTaskOperation::Cancel, None)
            .await?
        {
            NativeTaskOutput::Cancelled { accepted } => Ok(accepted),
            _ => Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_response(
                    "native cancellation returned an unexpected response",
                ),
            )),
        }
    }
}

fn observed_protocol_stream(
    stream: ProtocolStream,
    health: Arc<ModelHealthRegistry>,
    exact_model: String,
    provider_instance_name: String,
    started: Instant,
) -> ProtocolStream {
    let events = stream::unfold((stream.events, false), move |(mut events, terminal)| {
        let health = health.clone();
        let exact_model = exact_model.clone();
        let provider_instance_name = provider_instance_name.clone();
        async move {
            let event = events.next().await;
            let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
            match &event {
                Some(Ok(ProtocolEvent::Final(_))) => health.record_success(
                    &exact_model,
                    &provider_instance_name,
                    latency_ms,
                    now_ms(),
                ),
                Some(Err(error)) => health.record_failure(
                    &exact_model,
                    &provider_instance_name,
                    latency_ms,
                    health_failure_kind(error),
                    now_ms(),
                ),
                None if !terminal => health.record_failure(
                    &exact_model,
                    &provider_instance_name,
                    latency_ms,
                    HealthFailureKind::Transient,
                    now_ms(),
                ),
                _ => {}
            }
            event.map(|event| {
                let terminal = terminal || matches!(event, Ok(ProtocolEvent::Final(_)) | Err(_));
                (event, (events, terminal))
            })
        }
    });
    ProtocolStream {
        events: Box::pin(events),
    }
}

fn health_failure_kind(error: &ProtocolError) -> HealthFailureKind {
    if error.is_account_exhausted() {
        HealthFailureKind::Permanent
    } else if error.is_model_unavailable() {
        HealthFailureKind::ModelUnavailable
    } else if error.allows_model_failover() {
        HealthFailureKind::Transient
    } else {
        HealthFailureKind::Permanent
    }
}

fn resume_credential_kind(kind: CredentialKind) -> ResumeCredentialKind {
    match kind {
        CredentialKind::Bearer => ResumeCredentialKind::Bearer,
        CredentialKind::NamedHeader => ResumeCredentialKind::NamedHeader,
        CredentialKind::FalKey => ResumeCredentialKind::FalKey,
        CredentialKind::GlmJwt => ResumeCredentialKind::GlmJwt,
    }
}

fn credential_fingerprint(reference: &str) -> String {
    Sha256::digest(reference.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_failure_kind_scopes_account_exhaustion_and_model_capability_errors() {
        let exhausted = ProtocolError::new(
            ProtocolErrorKind::Transport,
            "OpenAI 1113: 余额不足或无可用资源包,请充值。",
        );
        assert_eq!(
            health_failure_kind(&exhausted),
            HealthFailureKind::Permanent
        );

        let thinking_unsupported = ProtocolError::new(
            ProtocolErrorKind::Transport,
            "OpenAI 1210: 该模型始终思考，不支持关闭思考；请使用 low、high 或 max。",
        );
        assert_eq!(
            health_failure_kind(&thinking_unsupported),
            HealthFailureKind::ModelUnavailable
        );

        let timeout = ProtocolError::new(ProtocolErrorKind::Timeout, "timed out");
        assert_eq!(health_failure_kind(&timeout), HealthFailureKind::Transient);
    }
}
