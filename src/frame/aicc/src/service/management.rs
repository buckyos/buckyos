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
                let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
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
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
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
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
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
        handle_video_text_to_video,
        VideoTextToVideoRequest,
        VideoTextToVideoResponse,
        VideoTextToVideo
    );
    typed_inference_handler!(
        handle_video_image_to_video,
        VideoImageToVideoRequest,
        VideoImageToVideoResponse,
        VideoImageToVideo
    );
    typed_inference_handler!(
        handle_video_to_video,
        VideoToVideoRequest,
        VideoToVideoResponse,
        VideoToVideo
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
        let snapshot = self
            .mutate_settings(&caller, Some(expected_revision), move |settings| {
                for (provider_instance_name, weight) in &provider_weights {
                    if !weight.is_finite() || *weight < 0.0 {
                        return Err(invalid_request(
                            "provider weight must be finite and non-negative",
                        ));
                    }
                    if !settings
                        .providers
                        .iter()
                        .any(|provider| provider.provider_instance_name == *provider_instance_name)
                    {
                        return Err(invalid_request(
                            "provider weight references an unknown provider",
                        ));
                    }
                }
                settings
                    .session_config
                    .get_or_insert_with(Default::default)
                    .provider_weights = provider_weights;
                Ok(())
            })
            .await?;
        Ok(RoutingUpdateResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
            routing: snapshot.routing,
        })
    }

    async fn handle_query_quota(
        &self,
        request: QuotaQueryRequest,
        ctx: RPCContext,
    ) -> Result<QuotaQueryResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        self.quota.query_quota(&caller, request).await
    }

    async fn handle_query_usage(
        &self,
        mut request: QueryUsageRequest,
        ctx: RPCContext,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        if request.filters.tenant_ids.is_empty() {
            request.filters.tenant_ids.push(caller.tenant_id);
        } else if request.filters.tenant_ids != [caller.tenant_id.clone()] {
            return Err(RPCErrors::NoPermission(
                "usage query cannot cross tenant boundary".to_string(),
            ));
        }
        self.usage.query_usage(request).await
    }

    async fn handle_query_trace(
        &self,
        request: QueryRouteTraceRequest,
        ctx: RPCContext,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        self.usage.query_trace(&caller.tenant_id, request).await
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
        _request: ProviderListRequest,
        ctx: RPCContext,
    ) -> Result<ProviderListResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
        Ok(ProviderListResponse {
            providers: snapshot.providers,
            settings_revision: snapshot.settings_revision,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_provider_health(
        &self,
        request: ProviderHealthRequest,
        ctx: RPCContext,
    ) -> Result<ProviderHealthResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        let snapshot = self.runtime.capture().await?;
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
        Ok(ProviderRefreshModelsResponse {
            ok: true,
            provider_instance_name: request.provider_instance_name,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_list_models(
        &self,
        _request: ListModelsRequest,
        ctx: RPCContext,
    ) -> Result<Value, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_INFO).await?;
        Ok(self.runtime.capture().await?.models)
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
