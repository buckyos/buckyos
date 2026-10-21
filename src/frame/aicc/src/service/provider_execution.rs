use super::*;

pub(crate) struct RuntimeProviderExecutionPort {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
    resource_store: Arc<dyn ResourceStore>,
    url_fetcher: Arc<dyn UrlResourceFetcher>,
    storage: Arc<AiccStorage>,
    model_health: Arc<ModelHealthRegistry>,
    native_input_leases: tokio::sync::Mutex<HashMap<String, Vec<crate::resource::ResourceLease>>>,
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
            native_input_leases: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    async fn remember_provider_artifacts(
        storage: &AiccStorage,
        provider_instance_name: &str,
        protocol_adapter_id: &str,
        origin_provider: &str,
        context: &ResourceAccessContext,
        output: &mut ProtocolOutput,
    ) -> Result<(), ProtocolError> {
        let provider_artifact_refs = output.take_provider_artifact_refs();
        for artifact in &output.artifacts {
            let buckyos_api::ResourceRef::Url { url, .. } = &artifact.resource else {
                continue;
            };
            storage
                .remember_artifact_url_source(&ArtifactUrlSourceRecord {
                    url: url.clone(),
                    provider_instance_name: provider_instance_name.to_owned(),
                    protocol_adapter_id: protocol_adapter_id.to_owned(),
                    origin_provider: origin_provider.to_owned(),
                    artifact_id: provider_artifact_refs
                        .get(&artifact.name)
                        .map(|artifact_ref| artifact_ref.id.clone()),
                    content_digest: None,
                    expires_at_ms: provider_artifact_refs
                        .get(&artifact.name)
                        .and_then(|artifact_ref| artifact_ref.expires_at_ms),
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
        for (artifact_name, artifact_ref) in provider_artifact_refs {
            let Some(artifact) = output
                .artifacts
                .iter()
                .find(|artifact| artifact.name == artifact_name)
            else {
                continue;
            };
            let created_at_ms = now_ms() as i64;
            // An object-form artifact is worth caching by object id even when no
            // content digest was recorded. The object id is the object's own
            // identity, so the entry can be looked up later without reading the
            // payload back — which is the only way an object that was handed to
            // the Provider by value (rather than downloaded from it) can ever
            // produce a usable cache entry.
            if let buckyos_api::ResourceRef::NamedObject { obj_id } = &artifact.resource {
                storage
                    .remember_provider_artifact(&ProviderArtifactRecord {
                        key: ProviderArtifactKey::ObjectId(obj_id.to_string()),
                        provider_instance_name: provider_instance_name.to_owned(),
                        origin_provider: origin_provider.to_owned(),
                        tenant_id: context.tenant_id.clone(),
                        artifact_id: artifact_ref.id.clone(),
                        expires_at_ms: artifact_ref.expires_at_ms,
                        created_at_ms,
                    })
                    .await
                    .map_err(|_| {
                        ProtocolError::invalid_configuration(
                            "Provider artifact object registration failed",
                        )
                    })?;
            }
            let Some(content_digest) = artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("digest"))
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            storage
                .remember_provider_artifact(&ProviderArtifactRecord {
                    key: ProviderArtifactKey::ContentDigest(content_digest),
                    provider_instance_name: provider_instance_name.to_owned(),
                    origin_provider: origin_provider.to_owned(),
                    tenant_id: context.tenant_id.clone(),
                    artifact_id: artifact_ref.id,
                    expires_at_ms: artifact_ref.expires_at_ms,
                    created_at_ms,
                })
                .await
                .map_err(|_| {
                    ProtocolError::invalid_configuration("Provider artifact ID registration failed")
                })?;
        }
        Ok(())
    }

    async fn remember_call_provider_artifacts(
        &self,
        call: &ResolvedProviderCall,
        output: &mut ProtocolOutput,
    ) -> Result<(), ProtocolError> {
        if output.artifacts.is_empty() {
            return Ok(());
        }
        let context = call
            .resource_access_context
            .as_ref()
            .ok_or_else(|| ProtocolError::invalid_configuration("artifact context is missing"))?;
        Self::remember_provider_artifacts(
            self.storage.as_ref(),
            &call.provider_instance_name,
            &call.protocol_adapter_id,
            &call.context.state_coordinate.origin_provider,
            context,
            output,
        )
        .await
    }

    fn uses_provider_artifact_id(call: &ResolvedProviderCall) -> bool {
        call.context
            .resources
            .values()
            .any(|resource| resource.provider_artifact_id.is_some())
    }

    fn without_provider_artifact_ids(call: &ResolvedProviderCall) -> ResolvedProviderCall {
        let mut call = call.clone();
        for resource in call.context.resources.values_mut() {
            resource.provider_artifact_id = None;
        }
        call
    }

    fn should_retry_without_provider_artifact_id(error: &ProtocolError) -> bool {
        if error.kind != ProtocolErrorKind::InvalidRequest {
            return false;
        }
        let mut text = error.message.to_ascii_lowercase();
        if let Some(provider_code) = &error.provider_code {
            text.push(' ');
            text.push_str(&provider_code.to_ascii_lowercase());
        }
        let mentions_media_ref = [
            "artifact",
            "file",
            "video",
            "media",
            "uri",
            "url",
            "id",
            "reference",
        ]
        .iter()
        .any(|needle| text.contains(needle));
        let mentions_invalid_ref = [
            "invalid",
            "not found",
            "not_found",
            "expired",
            "expire",
            "unsupported",
            "does not exist",
            "missing",
            "unrecognized",
            "unknown",
            "malformed",
        ]
        .iter()
        .any(|needle| text.contains(needle));
        mentions_media_ref && mentions_invalid_ref
    }

    fn has_passthrough_url(call: &ResolvedProviderCall) -> bool {
        call.resource_requirements.iter().any(|requirement| {
            matches!(requirement.resource, buckyos_api::ResourceRef::Url { .. })
                && !call.context.resources.contains_key(
                    crate::resource::ResourceKey::from_ref(&requirement.resource).as_str(),
                )
        })
    }

    fn resource_input_form(&self, call: &ResolvedProviderCall) -> ResourceInputForm {
        self.codecs.resource_input_form(
            &call.protocol_adapter_id,
            &call.operation,
            call.api_type,
            call.execution_mode,
            &call
                .resource_requirements
                .iter()
                .map(|requirement| requirement.request_pointer.as_str())
                .collect::<Vec<_>>(),
        )
    }

    async fn url_fallback_target(
        &self,
        call: &ResolvedProviderCall,
    ) -> Option<NativeTaskUrlFallbackTarget> {
        if !Self::has_passthrough_url(call) {
            return None;
        }
        let public_named_object_base_url = self
            .runtime
            .capture()
            .await
            .settings
            .public_named_object_base_url
            .clone();
        Self::fallback_target_for_input_form(
            self.resource_input_form(call),
            public_named_object_base_url,
        )
    }

    fn fallback_target_for_input_form(
        input_form: ResourceInputForm,
        public_named_object_base_url: Option<String>,
    ) -> Option<NativeTaskUrlFallbackTarget> {
        match input_form {
            ResourceInputForm::UrlOrBytes => Some(NativeTaskUrlFallbackTarget::Bytes),
            ResourceInputForm::UrlOnly => public_named_object_base_url
                .map(|base_url| NativeTaskUrlFallbackTarget::PublicNamedObject { base_url }),
            ResourceInputForm::BytesOnly => None,
        }
    }

    fn should_retry_url_fallback(error: &ProtocolError) -> bool {
        if !matches!(
            error.kind,
            ProtocolErrorKind::InvalidRequest
                | ProtocolErrorKind::ProviderRejected
                | ProtocolErrorKind::InvalidResponse
        ) {
            return false;
        }
        let mut text = error.message.to_ascii_lowercase();
        if let Some(provider_code) = &error.provider_code {
            text.push(' ');
            text.push_str(&provider_code.to_ascii_lowercase());
        }
        [
            "url",
            "uri",
            "download",
            "fetch",
            "image input",
            "input image",
            "source image",
            "input resource",
            "media",
        ]
        .iter()
        .any(|needle| text.contains(needle))
    }

    pub(super) async fn fetch_artifact_url_resource(
        &self,
        tenant_id: &str,
        user_id: &str,
        url: &reqwest::Url,
        max_bytes: u64,
    ) -> Result<FetchedResource, ResourceError> {
        let mut reader = self
            .open_artifact_url_reader(tenant_id, user_id, url.as_str(), None)
            .await
            .map_err(|error| {
                ResourceError::new(
                    if error.code == AiccErrorCode::PolicyDenied {
                        ResourceFailure::Unauthorized
                    } else {
                        ResourceFailure::Unavailable
                    },
                    error.message,
                )
            })?;
        if reader
            .content_length
            .is_some_and(|content_length| content_length > max_bytes)
        {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "Provider artifact exceeds byte limit",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.body.next().await {
            let chunk = chunk.map_err(|_| {
                ResourceError::new(
                    ResourceFailure::Unavailable,
                    "Provider artifact read failed",
                )
            })?;
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::LimitExceeded,
                    "Provider artifact exceeds byte limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(FetchedResource {
            bytes,
            mime: reader.content_type,
            file_name: url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        })
    }

    async fn fetch_url_resource(
        &self,
        url: &str,
        access: &ResourceAccessContext,
        max_bytes: u64,
    ) -> Result<FetchedResource, ProtocolError> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| ProtocolError::invalid_request("resource URL is invalid"))?;
        let source = self
            .storage
            .artifact_url_source(url, now_ms() as i64)
            .await
            .map_err(|_| {
                ProtocolError::invalid_configuration("artifact URL source lookup failed")
            })?;
        if source.is_some() {
            return self
                .fetch_artifact_url_resource(&access.tenant_id, &access.caller_id, &parsed, max_bytes)
                .await
                .map_err(|error| {
                    let kind = match error.failure {
                        ResourceFailure::Unauthorized => ProtocolErrorKind::Authentication,
                        ResourceFailure::LimitExceeded => ProtocolErrorKind::InvalidRequest,
                        _ => ProtocolErrorKind::Transport,
                    };
                    ProtocolError::new(kind, error.message)
                });
        }
        self.url_fetcher
            .fetch(&parsed, max_bytes)
            .await
            .map_err(|error| ProtocolError::new(ProtocolErrorKind::Transport, error.to_string()))
    }

    async fn materialize_url_fallback(
        &self,
        requirements: &[ResourceRequirement],
        access: &ResourceAccessContext,
        context: &mut CodecContext,
        target: &NativeTaskUrlFallbackTarget,
    ) -> Result<(), ProtocolError> {
        let limits = ResourceLimits::default();
        let mut total_bytes = 0u64;
        for (index, requirement) in requirements.iter().enumerate() {
            let buckyos_api::ResourceRef::Url { url, mime_hint } = &requirement.resource else {
                continue;
            };
            let key = crate::resource::ResourceKey::from_ref(&requirement.resource);
            if context.resources.contains_key(key.as_str()) {
                continue;
            }
            let fetched = self
                .fetch_url_resource(url, access, limits.max_single_bytes)
                .await?;
            total_bytes = total_bytes
                .checked_add(fetched.bytes.len() as u64)
                .filter(|size| *size <= limits.max_total_bytes)
                .ok_or_else(|| {
                    ProtocolError::invalid_request("resource batch exceeds byte limit")
                })?;
            let mime = mime_hint
                .clone()
                .or(fetched.mime)
                .unwrap_or_else(|| DEFAULT_RESOURCE_MIME.to_owned());
            let materialized = match target {
                NativeTaskUrlFallbackTarget::Bytes => {
                    CodecMaterializedResource::new(fetched.bytes, mime, fetched.file_name)?
                }
                NativeTaskUrlFallbackTarget::PublicNamedObject { base_url } => {
                    let scope_fingerprint = credential_fingerprint(&format!(
                        "{}\0{}\0{}",
                        access.tenant_id, access.caller_id, access.request_id
                    ));
                    let manager = ResourceManager::new(
                        Arc::new(AuthenticatedResourceAuthorizer {
                            tenant_id: access.tenant_id.clone(),
                            caller_id: access.caller_id.clone(),
                            storage: self.storage.clone(),
                        }),
                        self.resource_store.clone(),
                        self.url_fetcher.clone(),
                        limits.clone(),
                    )
                    .map_err(|_| {
                        ProtocolError::invalid_configuration("artifact writer is unavailable")
                    })?;
                    let artifact = manager
                        .write_artifact(
                            access,
                            &fetched.bytes,
                            ArtifactSpec {
                                name: format!(
                                    "provider-url-fallback-{scope_fingerprint}-{}",
                                    index + 1
                                ),
                                mime: mime.clone(),
                                attributes: Map::new(),
                                embedding: None,
                            },
                        )
                        .await
                        .map_err(|error| ProtocolError::invalid_request(error.to_string()))?;
                    let buckyos_api::ResourceRef::NamedObject { obj_id } = artifact.resource else {
                        return Err(ProtocolError::invalid_configuration(
                            "URL fallback artifact did not produce a Named Object",
                        ));
                    };
                    self.storage
                        .remember_artifact_scope(
                            &obj_id.to_string(),
                            &access.tenant_id,
                            &access.caller_id,
                            None,
                            now_ms() as i64,
                        )
                        .await
                        .map_err(|_| {
                            ProtocolError::invalid_configuration(
                                "URL fallback artifact ownership write failed",
                            )
                        })?;
                    let public_url = ZoneNamedObjectUrlProvider::new(base_url.clone())
                        .object_url(&obj_id)
                        .ok_or_else(|| {
                            ProtocolError::invalid_configuration(
                                "public Named Object URL is unavailable",
                            )
                        })?;
                    CodecMaterializedResource::from_url(public_url, mime, fetched.file_name)?
                }
            };
            context.resources.insert(key.into_string(), materialized);
        }
        Ok(())
    }

    async fn call_with_url_fallback(
        &self,
        call: &ResolvedProviderCall,
        target: &NativeTaskUrlFallbackTarget,
    ) -> Result<ResolvedProviderCall, ProtocolError> {
        let valid = matches!(
            (self.resource_input_form(call), target),
            (
                ResourceInputForm::UrlOrBytes,
                NativeTaskUrlFallbackTarget::Bytes
            ) | (
                ResourceInputForm::UrlOnly,
                NativeTaskUrlFallbackTarget::PublicNamedObject { .. }
            )
        );
        if !valid {
            return Err(ProtocolError::invalid_request(
                "Provider protocol URL fallback target is invalid",
            ));
        }
        let access = call.resource_access_context.as_ref().ok_or_else(|| {
            ProtocolError::invalid_configuration("resource access context is missing")
        })?;
        let mut retry = call.clone();
        self.materialize_url_fallback(
            &call.resource_requirements,
            access,
            &mut retry.context,
            target,
        )
        .await?;
        Ok(retry)
    }

    pub(crate) async fn open_artifact_url_reader(
        &self,
        tenant_id: &str,
        user_id: &str,
        url: &str,
        artifact_id: Option<&str>,
    ) -> Result<crate::protocol::ArtifactUrlReader, AiccError> {
        let source = self
            .storage
            .artifact_url_source(url, now_ms() as i64)
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
        if source.user_id != user_id {
            return Err(AiccError::new(
                AiccErrorCode::PolicyDenied,
                "Provider artifact belongs to another user",
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
        let artifact_origin = reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| {
                let host = parsed.host_str()?;
                Some(match parsed.port() {
                    Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
                    None => format!("{}://{}", parsed.scheme(), host),
                })
            })
            .unwrap_or_else(|| "<invalid>".to_owned());
        let mut reader = provider
            .open_artifact_url_reader(self.codecs.as_ref(), url)
            .await
            .map_err(|error| {
                log::warn!(
                    "open Provider artifact failed: provider={} adapter={} artifact_origin={} error_kind={:?} http_status={:?} provider_code={:?} request_id={:?} message={}",
                    source.provider_instance_name,
                    source.protocol_adapter_id,
                    artifact_origin,
                    error.kind,
                    error.http_status,
                    error.provider_code,
                    error.request_id,
                    error.message
                );
                AiccError {
                    code: AiccErrorCode::ProviderError,
                    message: error.message,
                    provider_code: error.provider_code,
                    retriable: matches!(
                        error.kind,
                        ProtocolErrorKind::Timeout | ProtocolErrorKind::Transport
                    ),
                    details: None,
                }
            })?;
        if source.content_digest.is_none() {
            let storage = self.storage.clone();
            let completion_source = source.clone();
            reader.body = Box::pin(stream::unfold(
                (reader.body, Sha256::new(), false),
                move |(mut body, mut hasher, failed)| {
                    let storage = storage.clone();
                    let source = completion_source.clone();
                    async move {
                        if failed {
                            return None;
                        }
                        match body.next().await {
                            Some(Ok(bytes)) => {
                                hasher.update(&bytes);
                                Some((Ok(bytes), (body, hasher, false)))
                            }
                            Some(Err(error)) => Some((Err(error), (body, hasher, true))),
                            None => {
                                let content_digest = format!("sha256:{:x}", hasher.finalize());
                                if let Err(error) = storage
                                    .complete_artifact_url_digest(
                                        &source,
                                        &content_digest,
                                        now_ms() as i64,
                                    )
                                    .await
                                {
                                    log::warn!(
                                        "failed to persist Provider artifact digest: {error}"
                                    );
                                }
                                None
                            }
                        }
                    }
                },
            ));
        }
        Ok(reader)
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
                token: None,
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
        append_materialized_artifact_metadata(&mut output.value, &artifact);
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
        let manager = ResourceManager::new(
            Arc::new(AuthenticatedResourceAuthorizer {
                tenant_id: context.tenant_id.clone(),
                caller_id: context.caller_id.clone(),
                storage: self.storage.clone(),
                token: None,
            }),
            self.resource_store.clone(),
            self.url_fetcher.clone(),
            ResourceLimits::default(),
        )
        .map_err(|_| ProtocolError::invalid_configuration("artifact writer is unavailable"))?;

        let mut inline_payloads = Vec::new();
        collect_inline_base64_payloads(&output.value, &mut inline_payloads);
        for artifact in &output.artifacts {
            if let buckyos_api::ResourceRef::Base64 { data_base64, .. } = &artifact.resource {
                if !inline_payloads.contains(data_base64) {
                    inline_payloads.push(data_base64.clone());
                }
            }
        }

        for index in 0..output.artifacts.len() {
            let (old_resource, name, mime, bytes, previous_metadata) =
                match &output.artifacts[index].resource {
                    buckyos_api::ResourceRef::Base64 { mime, data_base64 } => {
                        let bytes = BASE64_STANDARD.decode(data_base64).map_err(|_| {
                            ProtocolError::invalid_response("inline artifact is not valid base64")
                        })?;
                        (
                            output.artifacts[index].resource.clone(),
                            output.artifacts[index].name.clone(),
                            mime.clone(),
                            bytes,
                            output.artifacts[index].metadata.clone(),
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
            if let Some(Value::Object(previous)) = previous_metadata {
                let metadata = artifact
                    .metadata
                    .get_or_insert_with(|| Value::Object(Map::new()));
                if let Some(metadata) = metadata.as_object_mut() {
                    for (name, value) in previous {
                        metadata.entry(name).or_insert(value);
                    }
                }
            }
            append_materialized_artifact_metadata(&mut output.value, &artifact);
            output.artifacts[index] = artifact;
        }
        let mut value_resources = Vec::new();
        collect_inline_base64_resource_refs(&output.value, &mut value_resources);
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
            append_materialized_artifact_metadata(&mut output.value, &artifact);
            output.artifacts.push(artifact);
        }
        assert_materialized_output_has_no_inline_payloads(&output, &inline_payloads)?;
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

    fn validate_decision_output(
        request: &buckyos_api::DecisionEvaluateRequest,
        origin_model: &str,
        output: ProtocolOutput,
    ) -> Result<ProtocolOutput, ProtocolError> {
        let answers: Vec<buckyos_api::DecisionAnswer> =
            serde_json::from_value(output.value.get("answers").cloned().unwrap_or(Value::Null))
                .map_err(|_| ProtocolError::invalid_response("decision answers are malformed"))?;
        request
            .validate_answers(&answers)
            .map_err(|e| ProtocolError::invalid_response(e.message))?;
        if output
            .value
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|model| model != origin_model)
        {
            return Err(ProtocolError::invalid_response(
                "decision response model does not match resolved identity",
            ));
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
                    provider_profile_id: provider.profile.provider_profile_id.clone(),
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

    async fn retry_native_url_fallback(
        &self,
        binding: &PinnedProviderTask,
        cancellation: &crate::protocol::Cancellation,
    ) -> Result<(NativeTaskHandle, NativeTaskResumeDescriptor), NativeTaskResumeError> {
        let fallback = binding
            .resume
            .as_ref()
            .and_then(|resume| resume.url_fallback.clone())
            .ok_or_else(|| {
                NativeTaskResumeError::Protocol(ProtocolError::invalid_request(
                    "native task has no URL fallback",
                ))
            })?;
        let canonical_request = buckyos_api::AiccCall::from_method_and_params(
            &fallback.method,
            fallback.canonical_params.clone(),
        )
        .map_err(|error| {
            NativeTaskResumeError::Protocol(ProtocolError::invalid_configuration(error.to_string()))
        })?;
        let requirements = crate::call::collect_resource_requirements(&fallback.canonical_params);
        let input_form = self.codecs.resource_input_form(
            &binding.protocol_adapter_id,
            &binding.operation,
            binding.api_type,
            ExecutionMode::NativeTask,
            &requirements
                .iter()
                .map(|requirement| requirement.request_pointer.as_str())
                .collect::<Vec<_>>(),
        );
        let valid = matches!(
            (input_form, &fallback.target),
            (
                ResourceInputForm::UrlOrBytes,
                NativeTaskUrlFallbackTarget::Bytes
            ) | (
                ResourceInputForm::UrlOnly,
                NativeTaskUrlFallbackTarget::PublicNamedObject { .. }
            )
        );
        if !valid {
            return Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_request("Provider protocol URL fallback target is invalid"),
            ));
        }
        let (mut context, parameters) = self.resume_context(binding).await?;
        let access = binding
            .resume
            .as_ref()
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?
            .resource_access_context
            .clone();
        self.materialize_url_fallback(&requirements, &access, &mut context, &fallback.target)
            .await
            .map_err(NativeTaskResumeError::Protocol)?;
        let input = CodecInput {
            canonical_request,
            resolved_parameters: parameters,
        };
        let request = self
            .codecs
            .encode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(&input),
                    resolved_parameters: &input.resolved_parameters,
                    context: &context,
                },
            )
            .map_err(NativeTaskResumeError::Protocol)?;
        let transport =
            Self::transport(&context.limits).map_err(NativeTaskResumeError::Protocol)?;
        let response = Self::send_cancelable(cancellation, transport.send(request))
            .await
            .map_err(NativeTaskResumeError::Protocol)?;
        let output = self
            .codecs
            .decode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                NativeTaskOperation::Submit,
                response,
            )
            .await
            .map_err(NativeTaskResumeError::Protocol)?;
        let NativeTaskOutput::Submitted(handle) = output else {
            return Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_response("native URL fallback submit returned no task"),
            ));
        };
        let mut resume = binding
            .resume
            .clone()
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        resume.url_fallback = None;
        Ok((handle, resume))
    }

    async fn start_once(
        &self,
        call: &ResolvedProviderCall,
        cancellation: &crate::protocol::Cancellation,
    ) -> Result<ProviderExecution, ProviderStartFailure> {
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
                let mut response = Self::send_cancelable(cancellation, transport.send(request))
                    .await
                    .map_err(ProviderStartFailure::after_accept)?;
                let continuation = self.codecs.continuation_request(
                    &call.protocol_adapter_id, &call.operation, call.api_type,
                    &response, &call.input, &call.context,
                ).map_err(ProviderStartFailure::after_accept)?;
                let mut continuation_usage = None;
                if let Some((request, usage)) = continuation {
                    continuation_usage = usage;
                    response = Self::send_cancelable(cancellation, transport.send(request))
                        .await.map_err(ProviderStartFailure::after_accept)?;
                    if self.codecs.continuation_request(
                        &call.protocol_adapter_id, &call.operation, call.api_type,
                        &response, &call.input, &call.context,
                    ).map_err(ProviderStartFailure::after_accept)?.is_some() {
                        return Err(ProviderStartFailure::after_accept(ProtocolError::invalid_response(
                            "Provider exceeded the observation handshake limit",
                        )));
                    }
                }
                let decoded = self
                    .codecs
                    .decode_with_input(
                        &call.protocol_adapter_id,
                        &call.operation,
                        call.api_type,
                        response,
                        &call.input,
                    )
                    .await
                    .map_err(ProviderStartFailure::after_accept)?;
                match decoded {
                    crate::protocol::ProtocolExecution::Immediate(output) => {
                        let mut output = output;
                        if let Some(usage) = continuation_usage {
                            merge_continuation_usage(output.usage.get_or_insert_with(Default::default), usage)
                                .map_err(ProviderStartFailure::after_accept)?;
                        }
                        if let AiccCall::DecisionEvaluate(request) = &call.input.canonical_request {
                            let output = Self::validate_decision_output(
                                request,
                                &call.origin_model_id,
                                output,
                            )
                            .map_err(ProviderStartFailure::after_accept)?;
                            return Ok(ProviderExecution::Immediate(output));
                        }
                        crate::protocol::bind_provider_state_source(
                            &mut output.value,
                            &call.context.state_coordinate,
                        );
                        if is_video_api(call.api_type) {
                            let parameters = &call.input.resolved_parameters;
                            if let Some(images) = reference_image_count(parameters) {
                                let usage = output.usage.get_or_insert_with(Default::default);
                                usage.reference_images = usage.reference_images.or(Some(images));
                            }
                            crate::protocol::finish_video_output(
                                &mut output,
                                requested_video_output(parameters).as_ref(),
                            );
                        }
                        let output = self
                            .materialize_embedding_output(call, output)
                            .await
                            .map_err(ProviderStartFailure::after_accept)?;
                        let mut output = self
                            .materialize_inline_artifact_output(call, output)
                            .await
                            .map_err(ProviderStartFailure::after_accept)?;
                        self.remember_call_provider_artifacts(call, &mut output)
                            .await
                            .map_err(ProviderStartFailure::after_accept)?;
                        let output = Self::map_rerank_output(call, output)
                            .map_err(ProviderStartFailure::after_accept)?;
                        let output = Self::validate_computer_output(call, output)
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
                    Self::send_cancelable(cancellation, transport.send_streaming(request))
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
                        if let ProtocolEvent::Final(output) = &mut event {
                            if !output.artifacts.is_empty() {
                                let context = resource_context.as_ref().ok_or_else(|| {
                                    ProtocolError::invalid_configuration(
                                        "artifact context is missing",
                                    )
                                })?;
                                Self::remember_provider_artifacts(
                                    storage.as_ref(),
                                    &provider_instance_name,
                                    &protocol_adapter_id,
                                    &source.origin_provider,
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
                let output_video_seconds = self
                    .codecs
                    .output_video_seconds(
                        &call.protocol_adapter_id,
                        &call.operation,
                        call.api_type,
                        &request,
                    )
                    .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
                // Everything the resume descriptor needs is checked before the
                // submit: once the Provider holds a task, only its handle may
                // be lost to a failure, never resubmitted.
                let resource_access_context =
                    call.resource_access_context.clone().ok_or_else(|| {
                        ProviderStartFailure::before_accept(
                            ProtocolError::invalid_configuration(
                                "native task resource context is missing",
                            ),
                            false,
                        )
                    })?;
                let max_poll_duration = self
                    .codecs
                    .native_task_max_poll_duration(
                        &call.protocol_adapter_id,
                        &call.operation,
                        call.api_type,
                    )
                    .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
                let response = Self::send_cancelable(cancellation, transport.send(request))
                    .await
                    .map_err(ProviderStartFailure::native_submit_unknown)?;
                let rejected = !response.status.is_success();
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
                    .map_err(|error| {
                        if rejected {
                            ProviderStartFailure::native_submit_rejected(error)
                        } else {
                            ProviderStartFailure::native_submit_unknown(error)
                        }
                    })?;
                let NativeTaskOutput::Submitted(handle) = output else {
                    return Err(ProviderStartFailure::native_submit_unknown(
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
                self.native_input_leases.lock().await.insert(
                    format!("{}:{}", call.provider_instance_name, handle.remote_task_id),
                    call.resource_leases.clone(),
                );
                let url_fallback = self
                    .url_fallback_target(call)
                    .await
                    .map(|target| {
                        call.input
                            .canonical_request
                            .to_params()
                            .map(|canonical_params| NativeTaskUrlFallback {
                                method: call.method.clone(),
                                canonical_params,
                                target,
                            })
                    })
                    .transpose()
                    .map_err(|_| {
                        ProviderStartFailure::after_accept(ProtocolError::invalid_configuration(
                            "native URL fallback request cannot be serialized",
                        ))
                    })?;
                Ok(ProviderExecution::NativeTask {
                    handle,
                    resume: NativeTaskResumeDescriptor {
                        base_url: call.context.base_url.clone(),
                        credential,
                        resource_access_context,
                        input_leases: call
                            .resource_leases
                            .iter()
                            .map(|lease| lease.spec.clone())
                            .collect(),
                        resolved_parameters: call.input.resolved_parameters.clone(),
                        output_video_seconds,
                        request_timeout_ms: call.context.limits.request_timeout.as_millis() as u64,
                        max_poll_duration_ms: max_poll_duration.as_millis() as u64,
                        active_poll_elapsed_ms: 0,
                        max_request_bytes: call.context.limits.max_request_bytes as u64,
                        max_response_bytes: call.context.limits.max_response_bytes as u64,
                        url_fallback,
                    },
                })
            }
        }
    }
}

fn is_video_api(api_type: buckyos_api::ApiType) -> bool {
    matches!(
        api_type,
        buckyos_api::ApiType::VideoGenerate
            | buckyos_api::ApiType::VideoEdit
            | buckyos_api::ApiType::VideoExtend
            | buckyos_api::ApiType::VideoUpscale
    )
}

fn video_plan(parameters: &BTreeMap<String, Value>) -> Option<crate::video::VideoPlan> {
    crate::video::VideoPlan::from_parameters(parameters).ok()
}

fn requested_video_output(
    parameters: &BTreeMap<String, Value>,
) -> Option<buckyos_api::VideoOutputSpec> {
    video_plan(parameters)?.requested_output
}

fn reference_image_count(parameters: &BTreeMap<String, Value>) -> Option<u64> {
    video_plan(parameters).map(|plan| {
        plan.inputs(buckyos_api::VideoConditionKind::ReferenceImage)
            .count() as u64
    })
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
        let mut attempted_call = call.clone();
        let url_fallback_target = self.url_fallback_target(&attempted_call).await;
        let mut result = self.start_once(&attempted_call, &cancellation).await;
        if Self::uses_provider_artifact_id(&attempted_call)
            && result.as_ref().err().is_some_and(|failure| {
                Self::should_retry_without_provider_artifact_id(&failure.error)
            })
        {
            log::warn!(
                "Provider rejected cached artifact reference; retrying once with raw resource bytes: provider={} model={} api_type={:?}",
                call.provider_instance_name,
                call.exact_model,
                call.api_type
            );
            attempted_call = Self::without_provider_artifact_ids(&attempted_call);
            result = self.start_once(&attempted_call, &cancellation).await;
        }
        if Self::has_passthrough_url(&attempted_call)
            && url_fallback_target.is_some()
            && result
                .as_ref()
                .err()
                .is_some_and(|failure| Self::should_retry_url_fallback(&failure.error))
        {
            let target = url_fallback_target
                .as_ref()
                .expect("fallback target checked");
            log::warn!(
                "Provider rejected URL resource; retrying once with fallback input: provider={} model={} api_type={:?} target={:?}",
                call.provider_instance_name,
                call.exact_model,
                call.api_type,
                target
            );
            match self.call_with_url_fallback(&attempted_call, target).await {
                Ok(retry_call) => {
                    attempted_call = retry_call;
                    result = self.start_once(&attempted_call, &cancellation).await;
                }
                Err(error) => result = Err(ProviderStartFailure::before_accept(error, false)),
            }
        }
        let result = match result {
            Ok(ProviderExecution::Stream(stream)) => {
                // Providers may fetch URL-backed input after the response stream
                // starts. Keep request-owned content until that stream is dropped.
                let input_leases = call.resource_leases.clone();
                let stream = ProtocolStream {
                    events: Box::pin(stream.events.map(move |event| {
                        let _retained_inputs = &input_leases;
                        event
                    })),
                };
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
                if failure.provider_accepted {
                    for lease in &call.resource_leases {
                        lease.keep_until_expiry();
                    }
                }
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
        let lease_key = format!(
            "{}:{}",
            binding.provider_instance_name,
            binding.remote_task_id.as_deref().unwrap_or_default()
        );
        let mut retained = self.native_input_leases.lock().await;
        if !retained.contains_key(&lease_key) {
            let mut leases = Vec::new();
            for spec in binding
                .resume
                .as_ref()
                .into_iter()
                .flat_map(|resume| &resume.input_leases)
            {
                let obj_id = ndn_lib::ObjId::new(&spec.obj_id).map_err(|_| {
                    NativeTaskResumeError::Protocol(ProtocolError::invalid_configuration(
                        "invalid retained input ID",
                    ))
                })?;
                if let Some(lease) = self
                    .resource_store
                    .retain(&obj_id, Some(spec))
                    .await
                    .map_err(|_| {
                        NativeTaskResumeError::Protocol(ProtocolError::invalid_configuration(
                            "retained input is unavailable",
                        ))
                    })?
                {
                    leases.push(lease);
                }
            }
            retained.insert(lease_key.clone(), leases);
        }
        drop(retained);
        let status = self
            .native_request(binding, NativeTaskOperation::Status, Some(&cancellation))
            .await;
        let status = match status {
            Err(NativeTaskResumeError::Protocol(error))
                if binding
                    .resume
                    .as_ref()
                    .is_some_and(|resume| resume.url_fallback.is_some())
                    && Self::should_retry_url_fallback(&error) =>
            {
                log::warn!(
                    "native Provider task rejected URL resource; resubmitting with fallback input: provider={} model={} api_type={:?}",
                    binding.provider_instance_name,
                    binding.exact_model,
                    binding.api_type
                );
                let (handle, resume) = self
                    .retry_native_url_fallback(binding, &cancellation)
                    .await?;
                return Ok(NativeTaskPoll::Rebound(handle, resume));
            }
            Err(error) => return Err(error),
            Ok(status) => status,
        };
        let result = match status {
            NativeTaskOutput::Status {
                state,
                result_ref,
                result_usage,
                result_artifacts,
                ..
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
                    NativeTaskOutput::Result(mut output) => {
                        merge_native_result_usage(
                            &mut output,
                            result_usage,
                            binding
                                .resume
                                .as_ref()
                                .and_then(|resume| resume.output_video_seconds)
                                .map(|seconds| seconds as f64),
                        );
                        if is_video_api(binding.api_type) {
                            let parameters = binding
                                .resume
                                .as_ref()
                                .map(|resume| &resume.resolved_parameters);
                            if let Some(parameters) = parameters {
                                if let Some(images) = reference_image_count(parameters) {
                                    let usage = output.usage.get_or_insert_with(Default::default);
                                    usage.reference_images =
                                        usage.reference_images.or(Some(images));
                                }
                            }
                            crate::protocol::finish_video_output(
                                &mut output,
                                parameters.and_then(requested_video_output).as_ref(),
                            );
                        }
                        let mut artifact_refs = binding.result_artifacts.clone();
                        artifact_refs.extend(result_artifacts);
                        output.bind_provider_artifact_refs(&artifact_refs);
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
                        let mut output = output;
                        Self::remember_provider_artifacts(
                            self.storage.as_ref(),
                            &binding.provider_instance_name,
                            &binding.protocol_adapter_id,
                            &binding.origin_provider,
                            &binding
                                .resume
                                .as_ref()
                                .ok_or(NativeTaskResumeError::CredentialUnavailable)?
                                .resource_access_context,
                            &mut output,
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
        if matches!(
            result,
            Ok(NativeTaskPoll::Complete(_)) | Ok(NativeTaskPoll::Failed(_))
        ) {
            self.native_input_leases.lock().await.remove(&lease_key);
        }
        result
    }

    async fn release_native_inputs(&self, binding: &PinnedProviderTask, outcome_unknown: bool) {
        let key = format!("{}:{}", binding.provider_instance_name, binding.remote_task_id.as_deref().unwrap_or_default());
        if let Some(leases) = self.native_input_leases.lock().await.remove(&key) {
            if outcome_unknown { for lease in leases { lease.keep_until_expiry(); } }
        }
    }

    async fn cancel_native(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<bool, NativeTaskResumeError> {
        match self
            .native_request(binding, NativeTaskOperation::Cancel, None)
            .await?
        {
            NativeTaskOutput::Cancelled { accepted } => {
                if accepted {
                    self.native_input_leases.lock().await.remove(&format!(
                        "{}:{}",
                        binding.provider_instance_name,
                        binding.remote_task_id.as_deref().unwrap_or_default()
                    ));
                }
                Ok(accepted)
            }
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

fn merge_native_result_usage(
    output: &mut ProtocolOutput,
    status_usage: Option<buckyos_api::AiUsage>,
    fallback_video_seconds: Option<f64>,
) {
    let reported_cost = status_usage.as_ref().and_then(|usage| usage.reported_cost);
    let settled_cost = status_usage.as_ref().and_then(|usage| usage.cost.clone());
    let video_seconds = status_usage
        .and_then(|usage| usage.video_seconds)
        .or(fallback_video_seconds);
    if reported_cost.is_none() && settled_cost.is_none() && video_seconds.is_none() {
        return;
    }
    let usage = output.usage.get_or_insert_with(Default::default);
    if usage.reported_cost.is_none() {
        usage.reported_cost = reported_cost;
    }
    if usage.cost.is_none() {
        usage.cost = settled_cost;
    }
    if usage.video_seconds.is_none() {
        usage.video_seconds = video_seconds;
    }
}

fn merge_continuation_usage(target: &mut buckyos_api::AiUsage, source: buckyos_api::AiUsage) -> Result<(), ProtocolError> {
    macro_rules! add {
        ($($field:ident),*) => {$(
            target.$field = match (target.$field, source.$field) {
                (None, None) => None,
                (left, right) => Some(left.unwrap_or(0).checked_add(right.unwrap_or(0))
                    .ok_or_else(|| ProtocolError::invalid_response("continuation usage overflow"))?),
            };
        )*};
    }
    add!(input_tokens, output_tokens, total_tokens, cache_read_input_tokens,
        cache_write_input_tokens, cache_write_1h_input_tokens, reasoning_tokens,
        audio_input_tokens, image_input_tokens, audio_output_tokens, image_output_tokens,
        image_units, request_units, characters);
    macro_rules! add_float {
        ($($field:ident),*) => {$(
            if source.$field.is_some() {
                let sum = target.$field.unwrap_or(0.0) + source.$field.unwrap_or(0.0);
                if !sum.is_finite() { return Err(ProtocolError::invalid_response("continuation usage overflow")); }
                target.$field = Some(sum);
            }
        )*};
    }
    add_float!(audio_seconds, video_seconds, reported_cost);
    if let Some(cost) = source.cost {
        if let Some(target_cost) = target.cost.as_mut() {
            if target_cost.currency != cost.currency {
                return Err(ProtocolError::invalid_response("continuation usage currency mismatch"));
            }
            target_cost.amount += cost.amount;
            if !target_cost.amount.is_finite() { return Err(ProtocolError::invalid_response("continuation cost overflow")); }
        } else { target.cost = Some(cost); }
    }
    Ok(())
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
    fn native_result_usage_keeps_result_values_and_fills_status_settlement() {
        let mut output = ProtocolOutput {
            value: Value::Null,
            usage: Some(buckyos_api::AiUsage {
                request_units: Some(1),
                ..Default::default()
            }),
            artifacts: Vec::new(),
        };
        merge_native_result_usage(
            &mut output,
            Some(buckyos_api::AiUsage {
                reported_cost: Some(0.5),
                cost: Some(buckyos_api::AiCost::new(0.5, "USD")),
                video_seconds: Some(8.0),
                ..Default::default()
            }),
            Some(10.0),
        );
        let usage = output.usage.as_ref().unwrap();
        assert_eq!(usage.request_units, Some(1));
        assert_eq!(usage.reported_cost, Some(0.5));
        assert_eq!(usage.cost, Some(buckyos_api::AiCost::new(0.5, "USD")));
        assert_eq!(usage.video_seconds, Some(8.0));

        let mut existing = ProtocolOutput {
            value: Value::Null,
            usage: Some(buckyos_api::AiUsage {
                reported_cost: Some(0.25),
                cost: Some(buckyos_api::AiCost::new(0.25, "CNY")),
                video_seconds: Some(4.0),
                ..Default::default()
            }),
            artifacts: Vec::new(),
        };
        merge_native_result_usage(
            &mut existing,
            Some(buckyos_api::AiUsage {
                reported_cost: Some(0.5),
                cost: Some(buckyos_api::AiCost::new(0.5, "USD")),
                video_seconds: Some(8.0),
                ..Default::default()
            }),
            None,
        );
        let usage = existing.usage.unwrap();
        assert_eq!(usage.reported_cost, Some(0.25));
        assert_eq!(usage.cost, Some(buckyos_api::AiCost::new(0.25, "CNY")));
        assert_eq!(usage.video_seconds, Some(4.0));

        let mut empty = ProtocolOutput {
            value: Value::Null,
            usage: None,
            artifacts: Vec::new(),
        };
        merge_native_result_usage(&mut empty, None, None);
        assert!(empty.usage.is_none());
    }

    #[test]
    fn artifact_materialization_does_not_recollect_replaced_value_resource() {
        let inline = buckyos_api::ResourceRef::Base64 {
            mime: "audio/mpeg".to_owned(),
            data_base64: "dGVzdA==".to_owned(),
        };
        let named = buckyos_api::ResourceRef::NamedObject {
            obj_id: ndn_lib::ObjId::new("cyfile:010203").unwrap(),
        };
        let mut value = serde_json::to_value(&inline).unwrap();

        replace_resource_ref_value(&mut value, &inline, &named).unwrap();

        let mut remaining = Vec::new();
        collect_inline_base64_resource_refs(&value, &mut remaining);
        assert!(remaining.is_empty());
    }

    #[test]
    fn decision_result_is_checked_before_success_including_alias_version_drift() {
        let request = buckyos_api::DecisionEvaluateRequest::new(
            "jev-latest@test",
            serde_json::json!({"kind":"url","url":"opaque state"}),
            vec![buckyos_api::DecisionQuestion::Boolean {
                id: "q".into(),
                instructions: serde_json::json!("Test"),
                criteria: None,
            }],
        );
        let output = |value| ProtocolOutput {
            value,
            usage: None,
            artifacts: Vec::new(),
        };
        let valid = serde_json::json!({"model":"jev-1.13.0","answers":[{"id":"q","type":"boolean","probability_true":0.8}]});
        assert!(RuntimeProviderExecutionPort::validate_decision_output(
            &request,
            "jev-1.13.0",
            output(valid.clone())
        )
        .is_ok());
        assert!(RuntimeProviderExecutionPort::validate_decision_output(
            &request,
            "jev-2.0.0",
            output(valid)
        )
        .is_err());
        for answers in [
            serde_json::json!([]),
            serde_json::json!([{"id":"q","type":"boolean","probability_true":1.1}]),
            serde_json::json!([{"id":"other","type":"boolean","probability_true":0.8}]),
        ] {
            assert!(RuntimeProviderExecutionPort::validate_decision_output(
                &request,
                "jev-1.13.0",
                output(serde_json::json!({"answers":answers}))
            )
            .is_err());
        }
    }

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

    #[test]
    fn artifact_reference_retry_detection_is_narrow() {
        let expired = ProtocolError::invalid_request("video id expired");
        assert!(RuntimeProviderExecutionPort::should_retry_without_provider_artifact_id(&expired));

        let missing = ProtocolError::invalid_request("file reference not found");
        assert!(RuntimeProviderExecutionPort::should_retry_without_provider_artifact_id(&missing));

        let context_length = ProtocolError::invalid_request("context_length_exceeded");
        assert!(
            !RuntimeProviderExecutionPort::should_retry_without_provider_artifact_id(
                &context_length
            )
        );

        let auth = ProtocolError::new(ProtocolErrorKind::Authentication, "invalid API key");
        assert!(!RuntimeProviderExecutionPort::should_retry_without_provider_artifact_id(&auth));
    }

    #[test]
    fn url_fallback_requires_an_explicit_resource_rejection() {
        for error in [
            ProtocolError::invalid_request("input image URL is inaccessible"),
            ProtocolError::new(
                ProtocolErrorKind::ProviderRejected,
                "failed to download source image",
            ),
        ] {
            assert!(RuntimeProviderExecutionPort::should_retry_url_fallback(
                &error
            ));
        }
        assert!(!RuntimeProviderExecutionPort::should_retry_url_fallback(
            &ProtocolError::new(ProtocolErrorKind::Timeout, "image URL timed out")
        ));
        assert!(!RuntimeProviderExecutionPort::should_retry_url_fallback(
            &ProtocolError::new(
                ProtocolErrorKind::ProviderRejected,
                "content policy rejected"
            )
        ));
    }

    #[test]
    fn url_fallback_target_follows_the_protocol_input_form() {
        let public_url = Some("https://zone.example/ndn".to_owned());
        assert_eq!(
            RuntimeProviderExecutionPort::fallback_target_for_input_form(
                ResourceInputForm::UrlOrBytes,
                public_url.clone(),
            ),
            Some(NativeTaskUrlFallbackTarget::Bytes)
        );
        assert_eq!(
            RuntimeProviderExecutionPort::fallback_target_for_input_form(
                ResourceInputForm::UrlOnly,
                public_url.clone(),
            ),
            Some(NativeTaskUrlFallbackTarget::PublicNamedObject {
                base_url: public_url.unwrap(),
            })
        );
        assert_eq!(
            RuntimeProviderExecutionPort::fallback_target_for_input_form(
                ResourceInputForm::UrlOnly,
                None,
            ),
            None
        );
        assert_eq!(
            RuntimeProviderExecutionPort::fallback_target_for_input_form(
                ResourceInputForm::BytesOnly,
                Some("https://zone.example/ndn".to_owned()),
            ),
            None
        );
    }

    #[test]
    fn provider_artifact_origin_comparison_ignores_paths_and_default_ports() {
        assert!(super::inference::same_url_origin(
            "https://dashscope.aliyuncs.com/api/v1/files/a.png?token=x",
            "https://dashscope.aliyuncs.com:443/api/v1"
        ));
        assert!(!super::inference::same_url_origin(
            "https://cdn.example.com/a.png?token=x",
            "https://dashscope.aliyuncs.com/api/v1"
        ));
    }
}
    #[test]
    fn continuation_usage_sums_both_calls_and_rejects_currency_changes() {
        let mut target = buckyos_api::AiUsage {input_tokens:Some(4),output_tokens:Some(3),total_tokens:Some(7),
            request_units:Some(1),cost:Some(buckyos_api::AiCost::new(0.01,"USD")),..Default::default()};
        let source = target.clone();
        merge_continuation_usage(&mut target, source).unwrap();
        assert_eq!(target.total_tokens, Some(14));
        assert_eq!(target.request_units, Some(2));
        assert_eq!(target.cost.as_ref().unwrap().amount, 0.02);
        assert!(merge_continuation_usage(&mut target,buckyos_api::AiUsage {
            cost:Some(buckyos_api::AiCost::new(1.0,"CNY")),..Default::default()}).is_err());
    }
