use super::*;

macro_rules! typed_inference_handler {
    ($method:ident, $request:ty, $response:ty, $variant:ident) => {
        fn $method<'life0, 'async_trait>(
            &'life0 self,
            request: $request,
            ctx: RPCContext,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<$response, RPCErrors>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let caller = self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?;
                self.invoke_typed(&caller, AiccCall::$variant(request))
                    .await
            })
        }
    };
}

#[async_trait]
impl AiccHandler for AiccService {
    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> Result<CancelResponse, RPCErrors> {
        let caller = self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?;
        let execution = self
            .execution
            .as_ref()
            .ok_or_else(|| RPCErrors::ReasonError("execution runtime is unavailable".into()))?;
        let accepted = execution
            .cancel(&caller.tenant_id, task_id)
            .await
            .map_err(|error| error.to_krpc_error())?;
        Ok(CancelResponse {
            task_id: task_id.to_string(),
            accepted,
        })
    }

    async fn handle_route_resolve(
        &self,
        request: RouteResolveRequest,
        ctx: RPCContext,
    ) -> Result<RouteResolveResponse, RPCErrors> {
        let caller = self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?;
        let inference = self.inference.as_ref().ok_or_else(|| {
            inference_error(
                AiccErrorCode::InternalError,
                "inference runtime is unavailable",
            )
        })?;
        inference.resolve_route(&caller, request).await
    }

    typed_inference_handler!(
        handle_chat_completions_create,
        LlmChatInvokeRequest,
        LlmChatInvokeResponse,
        ChatCompletionsCreate
    );
    typed_inference_handler!(
        handle_images_generate,
        TextToImageInvokeRequest,
        TextToImageInvokeResponse,
        ImagesGenerate
    );
    typed_inference_handler!(
        handle_helper_llm_chat,
        LlmChatHelperRequest,
        LlmChatInvokeResponse,
        HelperLlmChat
    );
    typed_inference_handler!(
        handle_helper_text_to_image,
        TextToImageHelperRequest,
        TextToImageInvokeResponse,
        HelperTextToImage
    );
    typed_inference_handler!(
        handle_helper_video_generate,
        VideoGenerateHelperRequest,
        VideoGenerateResponse,
        HelperVideoGenerate
    );
    typed_inference_handler!(
        handle_embedding_text,
        EmbeddingTextRequest,
        EmbeddingTextResponse,
        EmbeddingText
    );
    typed_inference_handler!(
        handle_embedding_multimodal,
        EmbeddingMultimodalRequest,
        EmbeddingMultimodalResponse,
        EmbeddingMultimodal
    );
    typed_inference_handler!(
        handle_decision_evaluate,
        DecisionEvaluateRequest,
        DecisionEvaluateResponse,
        DecisionEvaluate
    );
    typed_inference_handler!(handle_rerank, RerankRequest, RerankResponse, Rerank);
    typed_inference_handler!(
        handle_image_to_image,
        ImageToImageRequest,
        ImageToImageResponse,
        ImageToImage
    );
    typed_inference_handler!(
        handle_image_inpaint,
        ImageInpaintRequest,
        ImageInpaintResponse,
        ImageInpaint
    );
    typed_inference_handler!(
        handle_image_upscale,
        ImageUpscaleRequest,
        ImageUpscaleResponse,
        ImageUpscale
    );
    typed_inference_handler!(
        handle_image_background_remove,
        ImageBackgroundRemoveRequest,
        ImageBackgroundRemoveResponse,
        ImageBackgroundRemove
    );
    typed_inference_handler!(
        handle_vision_ocr,
        VisionOcrRequest,
        VisionOcrResponse,
        VisionOcr
    );
    typed_inference_handler!(
        handle_vision_caption,
        VisionCaptionRequest,
        VisionCaptionResponse,
        VisionCaption
    );
    typed_inference_handler!(
        handle_vision_detect,
        VisionDetectRequest,
        VisionDetectResponse,
        VisionDetect
    );
    typed_inference_handler!(
        handle_vision_segment,
        VisionSegmentRequest,
        VisionSegmentResponse,
        VisionSegment
    );
    typed_inference_handler!(
        handle_audio_text_to_speech,
        AudioTextToSpeechRequest,
        AudioTextToSpeechResponse,
        AudioTextToSpeech
    );
    typed_inference_handler!(
        handle_audio_speech_recognition,
        AudioSpeechRecognitionRequest,
        AudioSpeechRecognitionResponse,
        AudioSpeechRecognition
    );
    typed_inference_handler!(
        handle_audio_music,
        AudioMusicRequest,
        AudioMusicResponse,
        AudioMusic
    );
    typed_inference_handler!(
        handle_audio_enhance,
        AudioEnhanceRequest,
        AudioEnhanceResponse,
        AudioEnhance
    );
    typed_inference_handler!(
        handle_video_generate,
        VideoGenerateRequest,
        VideoGenerateResponse,
        VideoGenerate
    );
    typed_inference_handler!(
        handle_video_edit,
        VideoEditRequest,
        VideoEditResponse,
        VideoEdit
    );
    typed_inference_handler!(
        handle_video_extend,
        VideoExtendRequest,
        VideoExtendResponse,
        VideoExtend
    );
    typed_inference_handler!(
        handle_video_upscale,
        VideoUpscaleRequest,
        VideoUpscaleResponse,
        VideoUpscale
    );
    typed_inference_handler!(
        handle_computer_use,
        ComputerUseRequest,
        ComputerUseResponse,
        ComputerUse
    );

    async fn handle_reload_settings(
        &self,
        _request: ServiceReloadSettingsRequest,
        ctx: RPCContext,
    ) -> Result<ServiceReloadSettingsResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let _guard = self.settings_mutation.lock().await;
        let current = self.settings.load(&caller.token).await?;
        let prepared = self
            .runtime
            .prepare_settings(current.document.clone())
            .await?;
        if prepared.settings_revision() != current.document.revision {
            prepared.discard().await;
            return Err(RPCErrors::ReasonError(
                "runtime prepared a different settings revision".to_string(),
            ));
        }
        let snapshot = prepared.publish().await?;
        Ok(ServiceReloadSettingsResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
        })
    }

    async fn handle_get_routing(
        &self,
        _request: RoutingGetRequest,
        ctx: RPCContext,
    ) -> Result<RoutingGetResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
        Ok(RoutingGetResponse {
            settings_revision: snapshot.settings_revision,
            routing: snapshot.routing,
            command_status: snapshot.routing_command_status,
        })
    }

    async fn handle_update_routing(
        &self,
        request: RoutingUpdateRequest,
        ctx: RPCContext,
    ) -> Result<RoutingUpdateResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let expected_revision = request.settings_revision;
        let provider_weights = request.provider_weights;
        let routing_commands = request.routing_commands;
        if provider_weights.is_none() && routing_commands.is_none() {
            return Err(invalid_request(
                "routing update requires provider_weights or routing_commands",
            ));
        }
        if let Some(commands) = &routing_commands {
            for (index, command) in commands.iter().enumerate() {
                command.validate().map_err(|reason| {
                    AiccError::new(AiccErrorCode::InvalidRequest, reason).to_krpc_error()
                })?;
                if commands[..index]
                    .iter()
                    .any(|previous| previous.same_subject(command))
                {
                    return Err(invalid_request(
                        "routing commands must not repeat the same subject",
                    ));
                }
            }
        }
        let command_count = routing_commands.as_ref().map(Vec::len);
        let snapshot = self
            .mutate_settings(&caller, Some(expected_revision), move |settings| {
                if let Some(provider_weights) = &provider_weights {
                    for (provider_instance_name, weight) in provider_weights {
                        if !weight.is_finite() || *weight < 0.0 {
                            return Err(invalid_request(
                                "provider weight must be finite and non-negative",
                            ));
                        }
                        if !settings.providers.iter().any(|provider| {
                            provider.provider_instance_name == *provider_instance_name
                        }) {
                            return Err(invalid_request(
                                "provider weight references an unknown provider",
                            ));
                        }
                    }
                }
                let routing = settings.session_config.get_or_insert_with(Default::default);
                if let Some(provider_weights) = provider_weights {
                    routing.provider_weights = provider_weights;
                }
                if let Some(routing_commands) = routing_commands {
                    routing.routing_commands = routing_commands;
                }
                Ok(())
            })
            .await?;
        if let Some(count) = command_count {
            self.events.record(
                AiccEventLevel::Info,
                events::EVENT_ROUTING_COMMANDS_UPDATED,
                format!("routing adjustments updated ({count} active)"),
                json!({ "count": count, "settings_revision": snapshot.settings_revision }),
            );
        }
        Ok(RoutingUpdateResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
            routing: snapshot.routing,
        })
    }

    async fn handle_preview_routing(
        &self,
        request: RoutingPreviewRequest,
        ctx: RPCContext,
    ) -> Result<RoutingPreviewResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let inference = self.inference.as_ref().ok_or_else(|| {
            inference_error(
                AiccErrorCode::InternalError,
                "inference runtime is unavailable",
            )
        })?;
        inference.preview_routes(&caller, request).await
    }

    async fn handle_list_events(
        &self,
        request: EventsListRequest,
        ctx: RPCContext,
    ) -> Result<EventsListResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let (events, active_warnings) = self.events.list(request.limit.unwrap_or(50).min(200));
        Ok(EventsListResponse {
            events,
            active_warnings,
        })
    }

    async fn handle_quota_rpc(
        &self,
        method: &str,
        params: Value,
        ctx: RPCContext,
    ) -> Result<Value, RPCErrors> {
        let caller = match method {
            buckyos_api::quota_methods::QUERY => {
                self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?
            }
            buckyos_api::quota_methods::ACCOUNT_RECHARGE
            | buckyos_api::quota_methods::RECHARGE_GET
            | buckyos_api::quota_methods::VISIBILITY_GET
            | buckyos_api::quota_methods::VISIBILITY_UPDATE
            | buckyos_api::quota_methods::VISIBILITY_PREVIEW => {
                self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?
            }
            _ => self.authorize(&ctx, "read", RESOURCE_INFO).await?,
        };
        let snapshot = self.runtime.capture().await?;
        self.quota
            .handle(&caller, method, params, &admin_inventory(&snapshot))
            .await
    }

    async fn handle_query_usage(
        &self,
        mut request: QueryUsageRequest,
        ctx: RPCContext,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let manages = caller.is_admin();
        if request.zone_wide {
            if !manages {
                return Err(RPCErrors::NoPermission(
                    "only an administrator may query usage of the whole Zone".to_string(),
                ));
            }
        } else if request.filters.tenant_ids.is_empty() {
            request.filters.tenant_ids.push(caller.tenant_id.clone());
        } else if !manages && request.filters.tenant_ids != [caller.tenant_id.clone()] {
            return Err(RPCErrors::NoPermission(
                "usage query cannot cross tenant boundary".to_string(),
            ));
        }
        let response = self.usage.query_usage(request).await?;
        if manages {
            return Ok(response);
        }
        let snapshot = self.runtime.capture().await?;
        let access = self.access(&caller, &snapshot).await?;
        if !access.is_restricted() {
            return Ok(response);
        }
        let mut value = serde_json::to_value(response).map_err(to_rpc_error)?;
        redact_hidden(&mut value, &access, &admin_inventory(&snapshot));
        serde_json::from_value(value).map_err(to_rpc_error)
    }

    async fn handle_query_trace(
        &self,
        request: QueryRouteTraceRequest,
        ctx: RPCContext,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let mut response = self.usage.query_trace(&caller.tenant_id, request).await?;
        if caller.is_admin() {
            return Ok(response);
        }
        let snapshot = self.runtime.capture().await?;
        let access = self.access(&caller, &snapshot).await?;
        if access.is_restricted() {
            let inventory = admin_inventory(&snapshot);
            for trace in &mut response.traces {
                redact_hidden(trace, &access, &inventory);
            }
        }
        Ok(response)
    }

    async fn handle_provider_catalog(
        &self,
        _request: ProviderCatalogRequest,
        ctx: RPCContext,
    ) -> Result<ProviderCatalogResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        Ok(self.runtime.capture().await?.provider_catalog)
    }

    async fn handle_list_protocol_adapters(
        &self,
        _request: ProtocolAdapterListRequest,
        ctx: RPCContext,
    ) -> Result<ProtocolAdapterListResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        Ok(self.runtime.capture().await?.protocol_adapters)
    }

    async fn handle_validate_provider(
        &self,
        request: ProviderValidateRequest,
        ctx: RPCContext,
    ) -> Result<ProviderValidateResponse, RPCErrors> {
        self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        self.validator.validate(request).await
    }

    async fn handle_add_provider(
        &self,
        request: ProviderAddRequest,
        ctx: RPCContext,
    ) -> Result<ProviderAddResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let validation = self.validator.validate(validate_request(&request)).await?;
        if !validation.errors.is_empty() || !validation.error_details.is_empty() {
            return Err(RPCErrors::ReasonError(
                "provider validation failed; settings were not changed".to_string(),
            ));
        }
        let name = request.provider_instance_name.clone();
        let mutation_name = name.clone();
        let provider = provider_from_add(request, validation.resolved_protocol_adapter_id)?;
        let snapshot = self
            .mutate_settings(&caller, None, move |settings| {
                if settings
                    .providers
                    .iter()
                    .any(|existing| existing.provider_instance_name == mutation_name)
                {
                    return Err(RPCErrors::ReasonError(
                        "provider instance already exists".to_string(),
                    ));
                }
                settings.providers.push(provider);
                Ok(())
            })
            .await?;
        Ok(ProviderAddResponse {
            ok: true,
            provider_instance_name: name,
            settings_revision: snapshot.settings_revision,
            reload: reload_result(&snapshot),
        })
    }

    async fn handle_list_providers(
        &self,
        request: ProviderListRequest,
        ctx: RPCContext,
    ) -> Result<ProviderListResponse, RPCErrors> {
        let caller = self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
        let manages = caller.is_admin() || matches!(caller.principal, CallerPrincipal::System { .. });
        if request.view == Some(buckyos_api::AiccListView::Manage) {
            if !manages {
                return Err(RPCErrors::NoPermission(
                    "the management view requires an administrator".to_string(),
                ));
            }
            return Ok(ProviderListResponse {
                providers: snapshot.providers,
                settings_revision: snapshot.settings_revision,
                inventory_revision: snapshot.inventory_revision,
            });
        }
        let access = self.access(&caller, &snapshot).await?;
        let inventory = admin_inventory(&snapshot);
        let providers = snapshot
            .providers
            .into_iter()
            .filter(|provider| access.allows_provider(&provider.provider_instance_name))
            .map(|mut provider| {
                if access.is_restricted() {
                    provider.inventory.model_count = inventory
                        .providers
                        .get(&provider.provider_instance_name)
                        .map(|models| {
                            models
                                .iter()
                                .filter(|model| access.allows_model(model))
                                .count() as u64
                        })
                        .unwrap_or(0);
                }
                if !manages {
                    provider.inventory.unmatched_models.clear();
                    provider.inventory.unavailable_presets.clear();
                    provider.inventory.unpriced_models.clear();
                }
                provider
            })
            .collect();
        Ok(ProviderListResponse {
            providers,
            settings_revision: snapshot.settings_revision,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_provider_health(
        &self,
        request: ProviderHealthRequest,
        ctx: RPCContext,
    ) -> Result<ProviderHealthResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
        if !self
            .access(&caller, &snapshot)
            .await?
            .allows_model(&request.exact_model)
        {
            return Err(RPCErrors::ReasonError("exact model was not found".to_string()));
        }
        let health = snapshot
            .provider_health
            .get(&request.exact_model)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("exact model was not found".to_string()))?;
        Ok(ProviderHealthResponse { health })
    }

    async fn handle_update_provider(
        &self,
        request: ProviderUpdateRequest,
        ctx: RPCContext,
    ) -> Result<ProviderUpdateResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let current = self.settings.load(&caller.token).await?;
        if current.document.revision != request.settings_revision {
            return Err(conflict_error(
                request.settings_revision,
                current.document.revision,
            ));
        }
        let mut candidate = current
            .document
            .settings
            .providers
            .iter()
            .find(|provider| provider.provider_instance_name == request.provider_instance_name)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("provider instance was not found".into()))?;
        apply_provider_update(&mut candidate, request.clone());
        SettingsDocument::new(
            current.document.revision.saturating_add(1),
            AiccSettings {
                providers: vec![candidate.clone()],
                session_config: None,
                public_named_object_base_url: current
                    .document
                    .settings
                    .public_named_object_base_url
                    .clone(),
            },
        )
        .map_err(to_rpc_error)?;
        if candidate.enabled {
            let validation = self
                .validator
                .validate(validate_settings_provider(&candidate))
                .await?;
            if !validation.errors.is_empty() || !validation.error_details.is_empty() {
                return Err(RPCErrors::ReasonError(
                    "provider validation failed; settings were not changed".to_string(),
                ));
            }
        }
        let name = request.provider_instance_name.clone();
        let mutation_name = name.clone();
        let expected = request.settings_revision;
        let snapshot = self
            .mutate_settings(&caller, Some(expected), move |settings| {
                let provider = settings
                    .providers
                    .iter_mut()
                    .find(|provider| provider.provider_instance_name == mutation_name)
                    .ok_or_else(|| {
                        RPCErrors::ReasonError("provider instance was not found".into())
                    })?;
                apply_provider_update(provider, request);
                Ok(())
            })
            .await?;
        let provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.provider_instance_name == name)
            .map(serde_json::to_value)
            .transpose()
            .map_err(to_rpc_error)?;
        Ok(ProviderUpdateResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
            provider,
        })
    }

    async fn handle_delete_provider(
        &self,
        request: ProviderDeleteRequest,
        ctx: RPCContext,
    ) -> Result<ProviderDeleteResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let name = request.provider_instance_name;
        let response_name = name.clone();
        let snapshot = self
            .mutate_settings(&caller, None, move |settings| {
                let provider = settings
                    .providers
                    .iter()
                    .find(|provider| provider.provider_instance_name == name)
                    .ok_or_else(|| {
                        RPCErrors::ReasonError("provider instance was not found".into())
                    })?;
                if is_non_deletable_dynamic_login_provider(provider) {
                    return Err(invalid_request(
                        "dynamic login SN provider cannot be deleted",
                    ));
                }
                settings
                    .providers
                    .retain(|provider| provider.provider_instance_name != name);
                if let Some(routing) = settings.session_config.as_mut() {
                    routing.provider_weights.remove(&name);
                }
                Ok(())
            })
            .await?;
        Ok(ProviderDeleteResponse {
            ok: true,
            provider_instance_name: Some(response_name),
            settings_revision: Some(snapshot.settings_revision),
            reload: Some(reload_result(&snapshot)),
            reason: None,
        })
    }

    async fn handle_refresh_provider_models(
        &self,
        request: ProviderRefreshModelsRequest,
        ctx: RPCContext,
    ) -> Result<ProviderRefreshModelsResponse, RPCErrors> {
        self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let snapshot = self
            .runtime
            .refresh_provider(&request.provider_instance_name)
            .await?;
        let unmatched_count = snapshot
            .providers
            .iter()
            .find(|provider| provider.provider_instance_name == request.provider_instance_name)
            .map(|provider| provider.inventory.unmatched_models.len() as u64)
            .unwrap_or(0);
        Ok(ProviderRefreshModelsResponse {
            unmatched_count,
            ok: true,
            provider_instance_name: request.provider_instance_name,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_list_models(
        &self,
        request: ListModelsRequest,
        ctx: RPCContext,
    ) -> Result<Value, RPCErrors> {
        let caller = self.authorize_opened(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
        let access = self.access(&caller, &snapshot).await?;
        let mut models = snapshot.models;
        if request.view == Some(buckyos_api::AiccListView::Manage) {
            if !(caller.is_admin() || matches!(caller.principal, CallerPrincipal::System { .. })) {
                return Err(RPCErrors::NoPermission(
                    "the management view requires an administrator".to_string(),
                ));
            }
            mark_model_access(&mut models, &access);
        } else if access.is_restricted() {
            filter_models_value(&mut models, &access);
        }
        Ok(models)
    }

    async fn handle_driver_metadata_update_get(
        &self,
        ctx: RPCContext,
    ) -> Result<DriverMetadataUpdateView, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        self.metadata.get().await
    }

    async fn handle_driver_metadata_update_set(
        &self,
        request: DriverMetadataUpdateSetReq,
        ctx: RPCContext,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SETTINGS).await?;
        let _guard = self.settings_mutation.lock().await;
        let current = self.settings.load(&caller.token).await?;
        let next_revision = current.document.revision.saturating_add(1);
        let candidate =
            SettingsDocument::new(next_revision, current.document.settings.as_ref().clone())
                .map_err(to_rpc_error)?;
        let prepared = self.runtime.prepare_settings(candidate).await?;
        if prepared.expected_revision() != current.document.revision
            || prepared.settings_revision() != next_revision
        {
            prepared.discard().await;
            return Err(RPCErrors::ReasonError(
                "runtime settings revision changed while preparing metadata update".to_string(),
            ));
        }
        let response = match self
            .metadata
            .set(&caller.token, current.document.revision, request)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                prepared.discard().await;
                return Err(error);
            }
        };
        let snapshot = prepared.publish().await?;
        if snapshot.settings_revision != response.settings_revision {
            return Err(RPCErrors::ReasonError(
                "metadata update runtime revision does not match persistence".to_string(),
            ));
        }
        Ok(response)
    }
}

/// Each model of a management `models.list` says whether the caller may use it.
fn mark_model_access(models: &mut Value, access: &crate::visibility::Access) {
    for model in models
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let accessible = model
            .get("exact_model")
            .and_then(Value::as_str)
            .is_some_and(|exact_model| access.allows_model(exact_model));
        if let Some(model) = model.as_object_mut() {
            model.insert("accessible".to_string(), Value::Bool(accessible));
        }
    }
}

/// Keep only what the caller may use: exact models, their catalog Provider
/// entries and logical items pointing at them (§3.4, filtered server-side).
fn filter_models_value(models: &mut Value, access: &crate::visibility::Access) {
    let allowed = |value: &Value| {
        value
            .as_str()
            .is_some_and(|exact_model| access.allows_model(exact_model))
    };
    if let Some(list) = models.get_mut("models").and_then(Value::as_array_mut) {
        list.retain(|model| model.get("exact_model").is_some_and(allowed));
    }
    if let Some(directory) = models.get_mut("directory").and_then(Value::as_object_mut) {
        for items in directory.values_mut().filter_map(Value::as_array_mut) {
            items.retain(|item| {
                item.get("target")
                    .and_then(Value::as_str)
                    .is_none_or(|target| !target.contains('@') || access.allows_model(target))
            });
        }
    }
    for vendor in models
        .pointer_mut("/catalog/vendors")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for model in vendor
            .get_mut("models")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(providers) = model.get_mut("providers").and_then(Value::as_array_mut) {
                for provider in providers.iter_mut() {
                    if let Some(exact_models) =
                        provider.get_mut("exact_models").and_then(Value::as_array_mut)
                    {
                        exact_models.retain(allowed);
                    }
                }
                providers.retain(|provider| {
                    provider
                        .get("exact_models")
                        .and_then(Value::as_array)
                        .is_some_and(|models| !models.is_empty())
                });
            }
        }
    }
}
