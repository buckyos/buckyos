use crate::aicc_usage_log::{
    QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest, QueryUsageResponse,
};
use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

pub const AICC_SERVICE_UNIQUE_ID: &str = "aicc";
pub const AICC_SERVICE_SERVICE_NAME: &str = "aicc";
pub const AICC_SERVICE_SERVICE_PORT: u16 = 4040;

pub mod ai_methods {
    pub const ROUTE_RESOLVE: &str = "route.resolve";
    pub const CHAT_COMPLETIONS_CREATE: &str = "chat.completions.create";
    pub const IMAGES_GENERATE: &str = "images.generate";
    pub const HELPER_LLM_CHAT: &str = "helper.llm_chat";
    pub const HELPER_TEXT_TO_IMAGE: &str = "helper.text_to_image";

    pub const EMBEDDING_TEXT: &str = "embedding.text";
    pub const EMBEDDING_MULTIMODAL: &str = "embedding.multimodal";
    pub const RERANK: &str = "rerank";
    pub const IMAGE_IMG2IMG: &str = "image.img2img";
    pub const IMAGE_INPAINT: &str = "image.inpaint";
    pub const IMAGE_UPSCALE: &str = "image.upscale";
    pub const IMAGE_BG_REMOVE: &str = "image.bg_remove";
    pub const VISION_OCR: &str = "vision.ocr";
    pub const VISION_CAPTION: &str = "vision.caption";
    pub const VISION_DETECT: &str = "vision.detect";
    pub const VISION_SEGMENT: &str = "vision.segment";
    pub const AUDIO_TTS: &str = "audio.tts";
    pub const AUDIO_ASR: &str = "audio.asr";
    pub const AUDIO_MUSIC: &str = "audio.music";
    pub const AUDIO_ENHANCE: &str = "audio.enhance";
    pub const VIDEO_TXT2VIDEO: &str = "video.txt2video";
    pub const VIDEO_IMG2VIDEO: &str = "video.img2video";
    pub const VIDEO_VIDEO2VIDEO: &str = "video.video2video";
    pub const VIDEO_EXTEND: &str = "video.extend";
    pub const VIDEO_UPSCALE: &str = "video.upscale";
    pub const AGENT_COMPUTER_USE: &str = "agent.computer_use";

    pub const CANCEL: &str = "cancel";
    pub const SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
    pub const QUOTA_QUERY: &str = "quota.query";
    pub const USAGE_QUERY: &str = "usage.query";
    pub const TRACE_QUERY: &str = "trace.query";
    pub const PROVIDER_CATALOG: &str = "provider.catalog";
    pub const PROTOCOL_ADAPTER_LIST: &str = "protocol_adapter.list";
    pub const PROVIDER_VALIDATE: &str = "provider.validate";
    pub const PROVIDER_ADD: &str = "provider.add";
    pub const PROVIDER_LIST: &str = "provider.list";
    pub const PROVIDER_HEALTH: &str = "provider.health";
    pub const PROVIDER_UPDATE: &str = "provider.update";
    pub const PROVIDER_DELETE: &str = "provider.delete";
    pub const PROVIDER_REFRESH_MODELS: &str = "provider.refresh_models";
    pub const MODELS_LIST: &str = "models.list";
    pub const DRIVER_METADATA_UPDATE_GET: &str = "driver_metadata_update.get";
    pub const DRIVER_METADATA_UPDATE_SET: &str = "driver_metadata_update.set";

    pub fn is_ai_method(method: &str) -> bool {
        matches!(
            method,
            CHAT_COMPLETIONS_CREATE
                | IMAGES_GENERATE
                | EMBEDDING_TEXT
                | EMBEDDING_MULTIMODAL
                | RERANK
                | IMAGE_IMG2IMG
                | IMAGE_INPAINT
                | IMAGE_UPSCALE
                | IMAGE_BG_REMOVE
                | VISION_OCR
                | VISION_CAPTION
                | VISION_DETECT
                | VISION_SEGMENT
                | AUDIO_TTS
                | AUDIO_ASR
                | AUDIO_MUSIC
                | AUDIO_ENHANCE
                | VIDEO_TXT2VIDEO
                | VIDEO_IMG2VIDEO
                | VIDEO_VIDEO2VIDEO
                | VIDEO_EXTEND
                | VIDEO_UPSCALE
                | AGENT_COMPUTER_USE
        )
    }

    pub fn is_aicc_core_method(method: &str) -> bool {
        matches!(
            method,
            ROUTE_RESOLVE | HELPER_LLM_CHAT | HELPER_TEXT_TO_IMAGE
        ) || is_ai_method(method)
    }

    pub fn is_management_method(method: &str) -> bool {
        matches!(
            method,
            SERVICE_RELOAD_SETTINGS
                | QUOTA_QUERY
                | USAGE_QUERY
                | TRACE_QUERY
                | PROVIDER_CATALOG
                | PROTOCOL_ADAPTER_LIST
                | PROVIDER_VALIDATE
                | PROVIDER_ADD
                | PROVIDER_LIST
                | PROVIDER_HEALTH
                | PROVIDER_UPDATE
                | PROVIDER_DELETE
                | PROVIDER_REFRESH_MODELS
                | MODELS_LIST
                | DRIVER_METADATA_UPDATE_GET
                | DRIVER_METADATA_UPDATE_SET
        )
    }
}

#[cfg(test)]
mod canonical_contract_tests {
    use super::*;

    fn named_object() -> ResourceRef {
        ResourceRef::named_object(ObjId::new("chunk:123456").unwrap())
    }

    #[test]
    fn method_api_type_and_capability_are_distinct_contracts() {
        assert_eq!(
            serde_json::to_value(ApiType::ImageTextToImage).unwrap(),
            json!("image.txt2img")
        );
        assert_eq!(
            ApiType::ImageTextToImage.typed_method(),
            ai_methods::IMAGES_GENERATE
        );
        assert_eq!(ApiType::ImageTextToImage.capability(), Capability::Image);
        assert_ne!(
            ApiType::ImageTextToImage.typed_method(),
            serde_json::to_value(ApiType::ImageTextToImage)
                .unwrap()
                .as_str()
                .unwrap()
        );
    }

    #[test]
    fn model_requirement_uses_canonical_capability_names() {
        assert_eq!(features::TOOL_CALL, "tool_call");
        assert_eq!(features::JSON_SCHEMA, "json_schema");

        let requirement = ModelRequirement {
            tool_call: true,
            json_schema: true,
            ..ModelRequirement::default()
        };
        assert_eq!(
            requirement.feature_names(),
            vec!["tool_call".to_string(), "json_schema".to_string()]
        );
        assert!(requirement.requires_feature("tool_call"));
        assert!(requirement.requires_feature("json_schema"));
        assert!(!requirement.requires_feature("tool_calling"));
        assert!(!requirement.requires_feature("json_output"));
    }

    #[test]
    fn canonical_ir_round_trips_without_losing_opaque_state() {
        let message = AiMessage::new(
            AiRole::Assistant,
            vec![
                AiContent::text("answer"),
                AiContent::Thinking {
                    summary: Some("summary".to_string()),
                    text: None,
                    provider_metadata: Some(json!({"signature": "opaque"})),
                },
                AiContent::ProviderState {
                    provider: "openai".to_string(),
                    value: json!({"type": "reasoning", "id": "rs_1"}),
                },
            ],
        );
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(serde_json::from_value::<AiMessage>(value).unwrap(), message);

        let resource = named_object();
        let value = serde_json::to_value(&resource).unwrap();
        assert_eq!(value["kind"], "named_object");
        assert_eq!(
            serde_json::from_value::<ResourceRef>(value).unwrap(),
            resource
        );
    }

    #[test]
    fn typed_requests_round_trip_and_reject_unknown_fields() {
        let request = EmbeddingTextRequest::new(
            "text-embedding-3-large@openai_primary",
            vec![EmbeddingTextItem::Text {
                text: "hello".to_string(),
                id: Some("item-1".to_string()),
            }],
        );
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(EmbeddingTextRequest::from_json(value).unwrap(), request);
        assert!(EmbeddingTextRequest::from_json(json!({
            "exact_model": "m@p",
            "items": [],
            "payload": {}
        }))
        .is_err());
        assert!(VideoImageToVideoRequest::from_json(json!({
            "exact_model": "m@p",
            "image": {"kind": "named_object", "obj_id": "chunk:123456"}
        }))
        .is_err());
        assert!(EmbeddingTextRequest::from_json(json!({
            "exact_model": "embedding.text",
            "items": []
        }))
        .is_err());
    }

    #[test]
    fn helpers_reject_exact_model_and_legacy_envelopes() {
        assert!(LlmChatHelperRequest::from_json(json!({
            "logical_model": "llm.chat",
            "exact_model": "gpt@provider",
            "messages": []
        }))
        .is_err());
        assert!(TextToImageHelperRequest::from_json(json!({
            "capability": "image",
            "model": {"alias": "image.txt2img"},
            "payload": {"text": "fox"}
        }))
        .is_err());
        assert!(RouteResolveRequest::from_json(json!({
            "api_type": "llm",
            "logical_model": "gpt@provider"
        }))
        .is_err());
    }

    #[test]
    fn stable_error_round_trips_across_task_boundary() {
        let error = AiccError {
            code: AiccErrorCode::ProviderError,
            message: "rate limited".to_string(),
            provider_code: Some("openai/rate_limit".to_string()),
            retriable: true,
            details: Some(json!({"request_id": "req-1"})),
        };
        assert_eq!(
            AiccError::from_task_data(&error.to_task_data()),
            Some(error.clone())
        );
        assert_eq!(error.to_task_event_data()["code"], "provider_error");
        let krpc_error = error.to_krpc_error();
        assert_eq!(AiccError::from_krpc_error(&krpc_error), Some(error));
    }

    #[test]
    fn canonical_reload_is_the_only_reload_method() {
        assert_eq!(
            ai_methods::SERVICE_RELOAD_SETTINGS,
            "service.reload_settings"
        );
        assert!(!ai_methods::is_aicc_core_method("reload_settings"));
        assert!(!ai_methods::is_aicc_core_method("service.reaload_settings"));
    }

    struct TypedHandler;

    #[async_trait]
    impl AiccHandler for TypedHandler {
        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<CancelResponse, RPCErrors> {
            Ok(CancelResponse::new(task_id.to_string(), true))
        }

        async fn handle_embedding_text(
            &self,
            request: EmbeddingTextRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<EmbeddingTextResponse, RPCErrors> {
            let mut response = EmbeddingTextResponse::new("task-1", AiMethodStatus::Succeeded);
            response.data.push(EmbeddingValue {
                index: 0,
                id: Some(request.items.len().to_string()),
                embedding: vec![0.25, 0.75],
                embedding_space_id: "test:2:cosine:v1".to_string(),
            });
            Ok(response)
        }
    }

    #[tokio::test]
    async fn typed_client_and_server_dispatch_without_generic_envelope() {
        let request = EmbeddingTextRequest::new(
            "embed@test",
            vec![EmbeddingTextItem::Text {
                text: "hello".to_string(),
                id: None,
            }],
        );
        let client = AiccClient::new_in_process(Box::new(TypedHandler));
        let response = client.embedding_text(request.clone()).await.unwrap();
        assert_eq!(response.data[0].embedding, vec![0.25, 0.75]);

        let server = AiccServerHandler::new(TypedHandler);
        let response = server
            .handle_rpc_call(
                RPCRequest {
                    method: ai_methods::EMBEDDING_TEXT.to_string(),
                    params: serde_json::to_value(request).unwrap(),
                    seq: 7,
                    token: None,
                    trace_id: Some("trace-1".to_string()),
                },
                "127.0.0.1".parse().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.seq, 7);

        let legacy_reload = server
            .handle_rpc_call(
                RPCRequest {
                    method: "reload_settings".to_string(),
                    params: json!({}),
                    seq: 8,
                    token: None,
                    trace_id: None,
                },
                "127.0.0.1".parse().unwrap(),
            )
            .await;
        assert!(matches!(legacy_reload, Err(RPCErrors::UnknownMethod(_))));
    }

    #[test]
    fn management_requests_are_strict_and_methods_are_canonical() {
        for method in [
            ai_methods::PROVIDER_CATALOG,
            ai_methods::PROTOCOL_ADAPTER_LIST,
            ai_methods::PROVIDER_VALIDATE,
            ai_methods::PROVIDER_ADD,
            ai_methods::PROVIDER_DELETE,
            ai_methods::PROVIDER_REFRESH_MODELS,
            ai_methods::USAGE_QUERY,
            ai_methods::TRACE_QUERY,
        ] {
            assert!(ai_methods::is_management_method(method));
        }

        let request = ProviderAddRequest::new(
            "openai-main",
            "cloud_api",
            "openai",
            "https://api.openai.com/v1",
            json!({"type": "bearer", "secret": "redacted"}),
        );
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(ProviderAddRequest::from_json(value).unwrap(), request);
        assert!(ProviderAddRequest::from_json(json!({
            "provider_instance_name": "openai-main",
            "provider_type": "cloud_api",
            "provider_profile_id": "openai",
            "base_url": "https://api.openai.com/v1",
            "credentials": {},
            "endpoint": "https://legacy.invalid"
        }))
        .is_err());
        assert!(QueryUsageRequest::from_json(json!({
            "time_range": {"kind": "last30d"},
            "filters": {"unknown_filter": "value"}
        }))
        .is_err());
        assert!(QueryRouteTraceRequest::from_json(json!({"unknown": true})).is_err());
        assert!(serde_json::from_value::<ProviderDeleteResponse>(json!({
            "ok": false,
            "reason": "provider_not_found"
        }))
        .is_ok());
    }

    #[test]
    fn metadata_view_exposes_target_and_per_provider_applied_sequences() {
        let view = DriverMetadataUpdateView {
            enabled: true,
            source_url: Some("ndn://metadata.example/aicc".to_string()),
            source_configured: true,
            interval_secs: 900,
            metadata_target_seq: 42,
            providers: vec![DriverMetadataProviderStatus::new("openai-main", 41)],
            status: DriverMetadataUpdateStatus::Updating,
            active_revision: Some(42),
            last_attempt_at_ms: None,
            last_success_at_ms: None,
            last_error: None,
            consecutive_failures: 0,
        };
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["metadata_target_seq"], 42);
        assert_eq!(value["providers"][0]["metadata_applied_seq"], 41);
        assert_eq!(
            serde_json::from_value::<DriverMetadataUpdateView>(value).unwrap(),
            view
        );
        assert!(serde_json::from_value::<DriverMetadataUpdateView>(json!({
            "enabled": true,
            "source_configured": true,
            "interval_secs": 900,
            "status": "updating",
            "consecutive_failures": 0
        }))
        .is_err());
    }

    struct ManagementHandler;

    #[async_trait]
    impl AiccHandler for ManagementHandler {
        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<CancelResponse, RPCErrors> {
            Ok(CancelResponse::new(task_id.to_string(), true))
        }

        async fn handle_provider_catalog(
            &self,
            _request: ProviderCatalogRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProviderCatalogResponse, RPCErrors> {
            Ok(ProviderCatalogResponse::default())
        }

        async fn handle_list_protocol_adapters(
            &self,
            _request: ProtocolAdapterListRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProtocolAdapterListResponse, RPCErrors> {
            Ok(ProtocolAdapterListResponse::default())
        }

        async fn handle_validate_provider(
            &self,
            _request: ProviderValidateRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProviderValidateResponse, RPCErrors> {
            Ok(ProviderValidateResponse::default())
        }

        async fn handle_add_provider(
            &self,
            request: ProviderAddRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProviderAddResponse, RPCErrors> {
            Ok(ProviderAddResponse {
                ok: true,
                provider_instance_name: request.provider_instance_name,
                settings_revision: 1,
                reload: ProviderReloadResult {
                    ok: true,
                    providers_registered: 1,
                },
            })
        }

        async fn handle_delete_provider(
            &self,
            request: ProviderDeleteRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProviderDeleteResponse, RPCErrors> {
            Ok(ProviderDeleteResponse {
                ok: true,
                provider_instance_name: Some(request.provider_instance_name),
                settings_revision: Some(2),
                reload: None,
                reason: None,
            })
        }

        async fn handle_refresh_provider_models(
            &self,
            request: ProviderRefreshModelsRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<ProviderRefreshModelsResponse, RPCErrors> {
            Ok(ProviderRefreshModelsResponse {
                ok: true,
                provider_instance_name: request.provider_instance_name,
                inventory_revision: "inventory-1".to_string(),
            })
        }

        async fn handle_query_usage(
            &self,
            _request: QueryUsageRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<QueryUsageResponse, RPCErrors> {
            Ok(QueryUsageResponse::default())
        }

        async fn handle_query_trace(
            &self,
            _request: QueryRouteTraceRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<QueryRouteTraceResponse, RPCErrors> {
            Ok(QueryRouteTraceResponse::default())
        }
    }

    fn provider_validate_request() -> ProviderValidateRequest {
        ProviderValidateRequest::new(
            "cloud_api",
            "openai",
            "https://api.openai.com/v1",
            json!({"type": "bearer", "secret": "redacted"}),
        )
    }

    fn provider_add_request() -> ProviderAddRequest {
        ProviderAddRequest::new(
            "openai-main",
            "cloud_api",
            "openai",
            "https://api.openai.com/v1",
            json!({"type": "bearer", "secret": "redacted"}),
        )
    }

    #[tokio::test]
    async fn management_client_and_server_dispatch_all_canonical_methods() {
        let client = AiccClient::new_in_process(Box::new(ManagementHandler));
        client
            .provider_catalog(ProviderCatalogRequest::new())
            .await
            .unwrap();
        client
            .list_protocol_adapters(ProtocolAdapterListRequest::new())
            .await
            .unwrap();
        client
            .validate_provider(provider_validate_request())
            .await
            .unwrap();
        client.add_provider(provider_add_request()).await.unwrap();
        client
            .delete_provider(ProviderDeleteRequest::new("openai-main"))
            .await
            .unwrap();
        client
            .refresh_provider_models(ProviderRefreshModelsRequest::new("openai-main"))
            .await
            .unwrap();
        client
            .query_usage(QueryUsageRequest::new(crate::UsageQueryTimeRange::Last30d))
            .await
            .unwrap();
        client
            .query_trace(QueryRouteTraceRequest::new())
            .await
            .unwrap();

        let calls = [
            (
                ai_methods::PROVIDER_CATALOG,
                serde_json::to_value(ProviderCatalogRequest::new()).unwrap(),
            ),
            (
                ai_methods::PROTOCOL_ADAPTER_LIST,
                serde_json::to_value(ProtocolAdapterListRequest::new()).unwrap(),
            ),
            (
                ai_methods::PROVIDER_VALIDATE,
                serde_json::to_value(provider_validate_request()).unwrap(),
            ),
            (
                ai_methods::PROVIDER_ADD,
                serde_json::to_value(provider_add_request()).unwrap(),
            ),
            (
                ai_methods::PROVIDER_DELETE,
                serde_json::to_value(ProviderDeleteRequest::new("openai-main")).unwrap(),
            ),
            (
                ai_methods::PROVIDER_REFRESH_MODELS,
                serde_json::to_value(ProviderRefreshModelsRequest::new("openai-main")).unwrap(),
            ),
            (
                ai_methods::USAGE_QUERY,
                serde_json::to_value(QueryUsageRequest::new(crate::UsageQueryTimeRange::Last30d))
                    .unwrap(),
            ),
            (
                ai_methods::TRACE_QUERY,
                serde_json::to_value(QueryRouteTraceRequest::new()).unwrap(),
            ),
        ];
        let server = AiccServerHandler::new(ManagementHandler);
        for (seq, (method, params)) in calls.into_iter().enumerate() {
            let response = server
                .handle_rpc_call(
                    RPCRequest {
                        method: method.to_string(),
                        params,
                        seq: seq as u64,
                        token: None,
                        trace_id: Some(format!("management-{seq}")),
                    },
                    "127.0.0.1".parse().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.seq, seq as u64);
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub enum ApiType {
    #[serde(rename = "llm")]
    Llm,
    #[serde(rename = "embedding.text")]
    EmbeddingText,
    #[serde(rename = "embedding.multimodal")]
    EmbeddingMultimodal,
    #[serde(rename = "rerank")]
    Rerank,
    #[serde(rename = "image.txt2img")]
    ImageTextToImage,
    #[serde(rename = "image.img2img")]
    ImageImageToImage,
    #[serde(rename = "image.inpaint")]
    ImageInpaint,
    #[serde(rename = "image.upscale")]
    ImageUpscale,
    #[serde(rename = "image.bg_remove")]
    ImageBackgroundRemove,
    #[serde(rename = "vision.ocr")]
    VisionOcr,
    #[serde(rename = "vision.caption")]
    VisionCaption,
    #[serde(rename = "vision.detect")]
    VisionDetect,
    #[serde(rename = "vision.segment")]
    VisionSegment,
    #[serde(rename = "audio.tts")]
    AudioTextToSpeech,
    #[serde(rename = "audio.asr")]
    AudioSpeechRecognition,
    #[serde(rename = "audio.music")]
    AudioMusic,
    #[serde(rename = "audio.enhance")]
    AudioEnhance,
    #[serde(rename = "video.txt2video")]
    VideoTextToVideo,
    #[serde(rename = "video.img2video")]
    VideoImageToVideo,
    #[serde(rename = "video.video2video")]
    VideoToVideo,
    #[serde(rename = "video.extend")]
    VideoExtend,
    #[serde(rename = "video.upscale")]
    VideoUpscale,
    #[serde(rename = "agent.computer_use")]
    AgentComputerUse,
}

impl ApiType {
    pub fn capability(self) -> Capability {
        match self {
            Self::Llm => Capability::Llm,
            Self::EmbeddingText | Self::EmbeddingMultimodal => Capability::Embedding,
            Self::Rerank => Capability::Rerank,
            Self::ImageTextToImage
            | Self::ImageImageToImage
            | Self::ImageInpaint
            | Self::ImageUpscale
            | Self::ImageBackgroundRemove => Capability::Image,
            Self::VisionOcr | Self::VisionCaption | Self::VisionDetect | Self::VisionSegment => {
                Capability::Vision
            }
            Self::AudioTextToSpeech
            | Self::AudioSpeechRecognition
            | Self::AudioMusic
            | Self::AudioEnhance => Capability::Audio,
            Self::VideoTextToVideo
            | Self::VideoImageToVideo
            | Self::VideoToVideo
            | Self::VideoExtend
            | Self::VideoUpscale => Capability::Video,
            Self::AgentComputerUse => Capability::Agent,
        }
    }

    pub fn typed_method(self) -> &'static str {
        match self {
            Self::Llm => ai_methods::CHAT_COMPLETIONS_CREATE,
            Self::EmbeddingText => ai_methods::EMBEDDING_TEXT,
            Self::EmbeddingMultimodal => ai_methods::EMBEDDING_MULTIMODAL,
            Self::Rerank => ai_methods::RERANK,
            Self::ImageTextToImage => ai_methods::IMAGES_GENERATE,
            Self::ImageImageToImage => ai_methods::IMAGE_IMG2IMG,
            Self::ImageInpaint => ai_methods::IMAGE_INPAINT,
            Self::ImageUpscale => ai_methods::IMAGE_UPSCALE,
            Self::ImageBackgroundRemove => ai_methods::IMAGE_BG_REMOVE,
            Self::VisionOcr => ai_methods::VISION_OCR,
            Self::VisionCaption => ai_methods::VISION_CAPTION,
            Self::VisionDetect => ai_methods::VISION_DETECT,
            Self::VisionSegment => ai_methods::VISION_SEGMENT,
            Self::AudioTextToSpeech => ai_methods::AUDIO_TTS,
            Self::AudioSpeechRecognition => ai_methods::AUDIO_ASR,
            Self::AudioMusic => ai_methods::AUDIO_MUSIC,
            Self::AudioEnhance => ai_methods::AUDIO_ENHANCE,
            Self::VideoTextToVideo => ai_methods::VIDEO_TXT2VIDEO,
            Self::VideoImageToVideo => ai_methods::VIDEO_IMG2VIDEO,
            Self::VideoToVideo => ai_methods::VIDEO_VIDEO2VIDEO,
            Self::VideoExtend => ai_methods::VIDEO_EXTEND,
            Self::VideoUpscale => ai_methods::VIDEO_UPSCALE,
            Self::AgentComputerUse => ai_methods::AGENT_COMPUTER_USE,
        }
    }
}

pub fn validate_exact_model_name(value: &str) -> std::result::Result<(), AiccError> {
    let mut parts = value.split('@');
    let provider_model_id = parts.next().unwrap_or_default();
    let provider_instance_name = parts.next().unwrap_or_default();
    if provider_model_id.is_empty() || provider_instance_name.is_empty() || parts.next().is_some() {
        return Err(AiccError::new(
            AiccErrorCode::InvalidModelName,
            "exact_model must be `<provider_model_id>[:<variant>]@<provider_instance_name>`",
        ));
    }
    Ok(())
}

pub fn validate_logical_model_name(value: &str) -> std::result::Result<(), AiccError> {
    if value.trim().is_empty() || value.contains('@') {
        return Err(AiccError::new(
            AiccErrorCode::InvalidModelName,
            "logical_model must be a non-empty logical path without `@`",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Llm,
    Embedding,
    Rerank,
    Image,
    Vision,
    Audio,
    Video,
    Agent,
}

pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALL: &str = "tool_call";
    pub const JSON_SCHEMA: &str = "json_schema";
    pub const WEB_SEARCH: &str = "web_search";
    pub const VISION: &str = "vision";
    pub const IMAGE_GENERATION: &str = "image_generation";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmResponseFormatType {
    Text,
    Json,
    JsonObject,
    JsonSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LlmJsonSchema {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LlmResponseFormat {
    #[serde(rename = "type")]
    pub format_type: LlmResponseFormatType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<LlmJsonSchema>,
}

impl LlmResponseFormat {
    pub fn text() -> Self {
        Self {
            format_type: LlmResponseFormatType::Text,
            json_schema: None,
        }
    }

    pub fn json_object() -> Self {
        Self {
            format_type: LlmResponseFormatType::JsonObject,
            json_schema: None,
        }
    }

    pub fn json_schema(name: Option<String>, schema: Value, strict: Option<bool>) -> Self {
        Self {
            format_type: LlmResponseFormatType::JsonSchema,
            json_schema: Some(LlmJsonSchema {
                name,
                schema,
                strict,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceRef {
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_hint: Option<String>,
    },
    Base64 {
        mime: String,
        data_base64: String,
    },
    NamedObject {
        obj_id: ObjId,
    },
}

impl ResourceRef {
    pub fn url(url: String, mime_hint: Option<String>) -> Self {
        Self::Url { url, mime_hint }
    }

    pub fn base64(mime: String, data_base64: String) -> Self {
        Self::Base64 { mime, data_base64 }
    }

    pub fn named_object(obj_id: ObjId) -> Self {
        Self::NamedObject { obj_id }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiccErrorCode {
    InvalidRequest,
    InvalidMethod,
    SchemaValidationFailed,
    InvalidModelName,
    ResourceInvalid,
    NoProviderAvailable,
    NoCandidateModel,
    FallbackNotAllowed,
    ProviderStartFailed,
    ProviderError,
    Timeout,
    BudgetExceeded,
    PolicyDenied,
    IdempotencyConflict,
    Cancelled,
    InternalError,
}

impl AiccErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidMethod => "invalid_method",
            Self::SchemaValidationFailed => "schema_validation_failed",
            Self::InvalidModelName => "invalid_model_name",
            Self::ResourceInvalid => "resource_invalid",
            Self::NoProviderAvailable => "no_provider_available",
            Self::NoCandidateModel => "no_candidate_model",
            Self::FallbackNotAllowed => "fallback_not_allowed",
            Self::ProviderStartFailed => "provider_start_failed",
            Self::ProviderError => "provider_error",
            Self::Timeout => "timeout",
            Self::BudgetExceeded => "budget_exceeded",
            Self::PolicyDenied => "policy_denied",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::Cancelled => "cancelled",
            Self::InternalError => "internal_error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiccError {
    pub code: AiccErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub retriable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl AiccError {
    pub fn new(code: AiccErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            provider_code: None,
            retriable: false,
            details: None,
        }
    }

    pub fn to_krpc_error(&self) -> RPCErrors {
        let body = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"code\":\"internal_error\",\"message\":{:?}}}",
                self.message
            )
        });
        match self.code {
            AiccErrorCode::InvalidRequest
            | AiccErrorCode::InvalidMethod
            | AiccErrorCode::SchemaValidationFailed
            | AiccErrorCode::InvalidModelName => RPCErrors::ParseRequestError(body),
            AiccErrorCode::PolicyDenied => RPCErrors::NoPermission(body),
            _ => RPCErrors::ReasonError(body),
        }
    }

    pub fn to_task_data(&self) -> Value {
        json!({ "aicc": { "error": self } })
    }

    pub fn to_task_event_data(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "code": "internal_error",
                "message": self.message,
                "retriable": false
            })
        })
    }

    pub fn from_task_data(data: &Value) -> Option<Self> {
        serde_json::from_value(data.pointer("/aicc/error")?.clone()).ok()
    }

    pub fn from_krpc_error(error: &RPCErrors) -> Option<Self> {
        let body = match error {
            RPCErrors::ParseRequestError(body)
            | RPCErrors::ReasonError(body)
            | RPCErrors::NoPermission(body) => body,
            _ => return None,
        };
        serde_json::from_str(body).ok()
    }
}

impl std::fmt::Display for AiccError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AiccError {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelRequirement {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub image_generation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
}

impl ModelRequirement {
    pub fn set_feature_required(&mut self, feature: &str) {
        match feature {
            features::TOOL_CALL => self.tool_call = true,
            features::JSON_SCHEMA => self.json_schema = true,
            features::WEB_SEARCH => self.web_search = true,
            features::VISION => self.vision = true,
            features::IMAGE_GENERATION => self.image_generation = true,
            "streaming" => self.streaming = true,
            _ => {}
        }
    }

    pub fn requires_feature(&self, feature: &str) -> bool {
        match feature {
            features::TOOL_CALL => self.tool_call,
            features::JSON_SCHEMA => self.json_schema,
            features::WEB_SEARCH => self.web_search,
            features::VISION => self.vision,
            features::IMAGE_GENERATION => self.image_generation,
            "streaming" => self.streaming,
            _ => false,
        }
    }

    pub fn feature_names(&self) -> Vec<Feature> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push(features::TOOL_CALL.to_string());
        }
        if self.json_schema {
            features.push(features::JSON_SCHEMA.to_string());
        }
        if self.web_search {
            features.push(features::WEB_SEARCH.to_string());
        }
        if self.vision {
            features.push(features::VISION.to_string());
        }
        if self.image_generation {
            features.push(features::IMAGE_GENERATION.to_string());
        }
        features
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelDisable {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub image_generation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
}

impl ModelDisable {
    pub fn set_feature_disabled(&mut self, feature: &str) {
        match feature {
            features::TOOL_CALL => self.tool_call = true,
            features::JSON_SCHEMA => self.json_schema = true,
            features::WEB_SEARCH => self.web_search = true,
            features::VISION => self.vision = true,
            features::IMAGE_GENERATION => self.image_generation = true,
            "streaming" => self.streaming = true,
            _ => {}
        }
    }

    pub fn disables_feature(&self, feature: &str) -> bool {
        match feature {
            features::TOOL_CALL => self.tool_call,
            features::JSON_SCHEMA => self.json_schema,
            features::WEB_SEARCH => self.web_search,
            features::VISION => self.vision,
            features::IMAGE_GENERATION => self.image_generation,
            "streaming" => self.streaming,
            _ => false,
        }
    }

    pub fn feature_names(&self) -> Vec<Feature> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push(features::TOOL_CALL.to_string());
        }
        if self.json_schema {
            features.push(features::JSON_SCHEMA.to_string());
        }
        if self.web_search {
            features.push(features::WEB_SEARCH.to_string());
        }
        if self.vision {
            features.push(features::VISION.to_string());
        }
        if self.image_generation {
            features.push(features::IMAGE_GENERATION.to_string());
        }
        if let Some(tokens) = self.min_context_tokens {
            features.push(format!("min_context_tokens:{}", tokens));
        }
        features
    }
}

fn is_default_model_disable(disable: &ModelDisable) -> bool {
    disable == &ModelDisable::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelItem {
    pub target: String,
    #[serde(default = "default_model_item_weight")]
    pub weight: f64,
}

impl ModelItem {
    pub fn new(target: impl Into<String>, weight: f64) -> Self {
        Self {
            target: target.into(),
            weight,
        }
    }
}

fn default_model_item_weight() -> f64 {
    1.0
}

pub type LogicalItems = BTreeMap<String, ModelItem>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMergeMode {
    #[default]
    Inherit,
    Replace,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiccFallbackMode {
    Strict,
    Parent,
    TargetExact,
    TargetLogical,
    Disabled,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiccFallbackRule {
    pub mode: AiccFallbackMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiccSchedulerProfile {
    #[default]
    CostFirst,
    LatencyFirst,
    QualityFirst,
    Balanced,
    LocalFirst,
    StrictLocal,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LockedValue<T> {
    pub value: T,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
}

impl<'de, T> Deserialize<'de> for LockedValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum LockedValueSerde<T> {
            Raw(T),
            Object {
                value: T,
                #[serde(default)]
                locked: bool,
            },
        }

        match LockedValueSerde::deserialize(deserializer)? {
            LockedValueSerde::Raw(value) => Ok(Self {
                value,
                locked: false,
            }),
            LockedValueSerde::Object { value, locked } => Ok(Self { value, locked }),
        }
    }
}

impl<T> LockedValue<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            locked: false,
        }
    }

    pub fn locked(value: T) -> Self {
        Self {
            value,
            locked: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccPolicyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<LockedValue<AiccSchedulerProfile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_profiles: Option<LockedValue<AiccSchedulerProfileConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_only: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallback: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_exact_model_fallback: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_failover: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_estimated_cost_usd: Option<LockedValue<f64>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSchedulerProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balanced: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_local: Option<AiccSchedulerProfileWeights>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSchedulerProfileWeights {
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub latency: f64,
    #[serde(default)]
    pub reliability: f64,
    #[serde(default)]
    pub quality: f64,
    #[serde(default)]
    pub preference: f64,
    #[serde(default)]
    pub cache: f64,
    #[serde(default)]
    pub local: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccLogicalNodeOverlay {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, AiccLogicalNodeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<LogicalItems>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_overrides: Option<BTreeMap<String, ModelItemPatch>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<ModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<AiccFallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<AiccPolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSessionLogicalProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<AiccLogicalTreeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccLogicalTreeOverlay {
    pub path: String,
    #[serde(default, skip_serializing_if = "is_default_overlay_merge_mode")]
    pub merge_mode: OverlayMergeMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub items: LogicalItems,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub item_overrides: BTreeMap<String, ModelItemPatch>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<ModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<AiccFallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn is_default_overlay_merge_mode(mode: &OverlayMergeMode) -> bool {
    *mode == OverlayMergeMode::default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccRouteOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logical_tree: BTreeMap<String, AiccLogicalNodeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_profile: Option<AiccSessionLogicalProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logical_profiles: BTreeMap<String, AiccSessionLogicalProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_logical_profile: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub global_exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "is_default_aicc_policy_config")]
    pub policy: AiccPolicyConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
}

fn is_default_aicc_policy_config(policy: &AiccPolicyConfig) -> bool {
    policy == &AiccPolicyConfig::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoutePolicyProfile {
    Cheap,
    Fast,
    #[default]
    Balanced,
    Quality,
}

fn is_default_route_policy_profile(profile: &RoutePolicyProfile) -> bool {
    matches!(profile, RoutePolicyProfile::Balanced)
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutePolicy {
    #[serde(default, skip_serializing_if = "is_default_route_policy_profile")]
    pub profile: RoutePolicyProfile,
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_only: bool,
    #[serde(default = "default_allow_fallback")]
    pub allow_fallback: bool,
    #[serde(default = "default_runtime_failover")]
    pub runtime_failover: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explain: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_provider_instances: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_provider_instances: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
}

fn default_allow_fallback() -> bool {
    true
}

fn default_runtime_failover() -> bool {
    true
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self {
            profile: RoutePolicyProfile::Balanced,
            local_only: false,
            allow_fallback: true,
            runtime_failover: true,
            explain: false,
            allowed_provider_instances: Vec::new(),
            blocked_provider_instances: Vec::new(),
            max_cost_usd: None,
            max_latency_ms: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiToolSpec {
    #[serde(rename = "type", default = "default_tool_type")]
    pub tool_type: String,
    pub name: String,
    pub description: String,
    pub args_json_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

fn default_tool_type() -> String {
    "function".to_string()
}

pub fn value_to_object_map(value: Value) -> HashMap<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

/// IR-level role for a message in `AiMessage`. Provider lowering rewrites
/// `Tool` and `Developer` per §1.4 of the AiMessage 重构 design doc.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiRole {
    System,
    User,
    Assistant,
    /// IR-internal carrier role for tool results. Each adapter MUST rewrite
    /// into the provider's native form (function_call_output / tool message
    /// / nested user+tool_result block / etc.).
    Tool,
    /// OpenAI Responses native; other providers fold into nearest `System`
    /// or downgrade to `System` role.
    Developer,
}

impl AiRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Developer => "developer",
        }
    }
}

/// Strict content subset allowed inside `AiContent::ToolResult.content` —
/// excludes ToolUse / ToolResult / Thinking, which have no meaning nested
/// inside a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AiToolResultContent {
    Text {
        text: String,
    },
    Image {
        source: ResourceRef,
    },
    Document {
        source: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

impl AiToolResultContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn text_str(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }
}

/// Content block. Mirrors the Anthropic content-block model, generalized
/// enough to round-trip OpenAI Responses items and Gemini parts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AiContent {
    /// Plain text segment.
    Text { text: String },

    /// Image block; reuses `ResourceRef` (URL / base64 / named object).
    Image { source: ResourceRef },

    /// Long-document attachment (PDF / large text), mirrors Claude document API.
    Document {
        source: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },

    /// Assistant requesting a tool call.
    ToolUse {
        call_id: String,
        name: String,
        #[serde(default)]
        args: HashMap<String, Value>,
    },

    /// Tool result echoed back to the LLM, keyed by `call_id` of the
    /// originating `ToolUse`.
    ToolResult {
        call_id: String,
        content: Vec<AiToolResultContent>,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
    },

    /// Extended thinking / reasoning block. `summary` is OpenAI Responses
    /// reasoning summary; `text` is Claude thinking plaintext;
    /// `provider_metadata` holds per-provider signature/state bits that
    /// aren't worth a dedicated field.
    Thinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Provider-specific native item that needs to round-trip but cannot be
    /// abstracted across providers (OpenAI reasoning item id/encrypted_content,
    /// Claude server_tool_use / web_search_tool_result, etc.).
    ///
    /// `provider` is the stable owner/consumer namespace for the opaque item,
    /// not the native item's protocol or type name. Each adapter defines the
    /// provider namespaces it can restore; the rest are dropped.
    ProviderState { provider: String, value: Value },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum AiMessageError {
    #[error("block type `{block_type}` is not allowed for role `{role:?}`")]
    InvalidBlockForRole {
        role: AiRole,
        block_type: &'static str,
    },
    #[error("tool_use / tool_result missing call_id")]
    MissingCallId,
    #[error("tool_result content must not be empty")]
    EmptyToolResult,
    #[error("role `Tool` requires exactly one ToolResult block")]
    ToolRoleShape,
}

impl AiContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(source: ResourceRef) -> Self {
        Self::Image { source }
    }

    pub fn tool_use(
        call_id: impl Into<String>,
        name: impl Into<String>,
        args: HashMap<String, Value>,
    ) -> Self {
        Self::ToolUse {
            call_id: call_id.into(),
            name: name.into(),
            args,
        }
    }

    pub fn tool_result_text(
        call_id: impl Into<String>,
        text: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            content: vec![AiToolResultContent::text(text)],
            is_error,
        }
    }

    fn type_tag(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Document { .. } => "document",
            Self::ToolUse { .. } => "tool_use",
            Self::ToolResult { .. } => "tool_result",
            Self::Thinking { .. } => "thinking",
            Self::ProviderState { .. } => "provider_state",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: Vec<AiContent>,
}

impl AiMessage {
    /// Single text block constructor — covers ~90% of call sites
    /// (system prompts, plain user/assistant messages).
    pub fn text(role: AiRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![AiContent::text(text)],
        }
    }

    /// Construct from explicit blocks. Caller is responsible for `validate()`.
    pub fn new(role: AiRole, content: Vec<AiContent>) -> Self {
        Self { role, content }
    }

    /// Concatenate all `Text` blocks' `text`, joined by `\n`. Non-text
    /// blocks are skipped. Use this when you need a string-shaped view of
    /// the message (transcript rendering, logging).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let AiContent::Text { text } = block {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// Extract assistant tool-use blocks as normalized tool calls, preserving
    /// their order within the message.
    pub fn tool_calls(&self) -> Vec<AiToolCall> {
        self.content
            .iter()
            .filter_map(|block| match block {
                AiContent::ToolUse {
                    call_id,
                    name,
                    args,
                } => Some(AiToolCall {
                    name: name.clone(),
                    args: args.clone(),
                    call_id: call_id.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// First `Text` block's content, if any. Use this for "I used to read
    /// `&msg.content`" replacement sites.
    pub fn first_text(&self) -> Option<&str> {
        self.content.iter().find_map(|block| match block {
            AiContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }

    /// Human-readable debug rendering of every block. Used by transcript
    /// dumps and worklog text. Stable enough for snapshot tests.
    pub fn render_for_debug(&self) -> String {
        let mut out = String::new();
        for (idx, block) in self.content.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            match block {
                AiContent::Text { text } => out.push_str(text),
                AiContent::Image { source: _ } => out.push_str("[image]"),
                AiContent::Document { title, .. } => {
                    out.push_str("[document");
                    if let Some(t) = title {
                        out.push_str(": ");
                        out.push_str(t);
                    }
                    out.push(']');
                }
                AiContent::ToolUse { call_id, name, .. } => {
                    out.push_str(&format!("[tool_use name={name} call_id={call_id}]"));
                }
                AiContent::ToolResult {
                    call_id,
                    content,
                    is_error,
                } => {
                    out.push_str(&format!(
                        "[tool_result call_id={call_id}{}]",
                        if *is_error { " error" } else { "" }
                    ));
                    for c in content {
                        if let AiToolResultContent::Text { text } = c {
                            out.push('\n');
                            out.push_str(text);
                        }
                    }
                }
                AiContent::Thinking { summary, text, .. } => {
                    out.push_str("[thinking");
                    if let Some(s) = summary {
                        out.push_str(" summary=");
                        out.push_str(s);
                    }
                    if let Some(t) = text {
                        out.push('\n');
                        out.push_str(t);
                    }
                    out.push(']');
                }
                AiContent::ProviderState { provider, .. } => {
                    out.push_str(&format!("[provider_state provider={provider}]"));
                }
            }
        }
        out
    }

    /// Rough byte-length estimate used by `llm_compress` to budget context.
    /// Non-text blocks contribute a conservative constant (~256 bytes for
    /// Image/Document, ToolUse args measured via JSON).
    pub fn estimate_text_len(&self) -> usize {
        let mut total = 0;
        for block in &self.content {
            match block {
                AiContent::Text { text } => total += text.len(),
                AiContent::Image { .. } | AiContent::Document { .. } => total += 256,
                AiContent::ToolUse {
                    name,
                    call_id,
                    args,
                } => {
                    total += name.len() + call_id.len();
                    if let Ok(s) = serde_json::to_string(args) {
                        total += s.len();
                    }
                }
                AiContent::ToolResult {
                    content, call_id, ..
                } => {
                    total += call_id.len();
                    for c in content {
                        match c {
                            AiToolResultContent::Text { text } => total += text.len(),
                            AiToolResultContent::Image { .. }
                            | AiToolResultContent::Document { .. } => total += 256,
                        }
                    }
                }
                AiContent::Thinking { summary, text, .. } => {
                    if let Some(s) = summary {
                        total += s.len();
                    }
                    if let Some(t) = text {
                        total += t.len();
                    }
                }
                AiContent::ProviderState { value, .. } => {
                    if let Ok(s) = serde_json::to_string(value) {
                        total += s.len();
                    }
                }
            }
        }
        total
    }

    /// Validate role × content combinations per §1.1 of the design doc.
    /// Typed chat clients call this before the request leaves the AICC client.
    pub fn validate(&self) -> std::result::Result<(), AiMessageError> {
        match self.role {
            AiRole::System | AiRole::Developer => {
                for block in &self.content {
                    if !matches!(block, AiContent::Text { .. }) {
                        return Err(AiMessageError::InvalidBlockForRole {
                            role: self.role,
                            block_type: block.type_tag(),
                        });
                    }
                }
            }
            AiRole::User => {
                for block in &self.content {
                    match block {
                        AiContent::Text { .. }
                        | AiContent::Image { .. }
                        | AiContent::Document { .. } => {}
                        _ => {
                            return Err(AiMessageError::InvalidBlockForRole {
                                role: self.role,
                                block_type: block.type_tag(),
                            });
                        }
                    }
                }
            }
            AiRole::Assistant => {
                for block in &self.content {
                    match block {
                        AiContent::Text { .. }
                        | AiContent::Image { .. }
                        | AiContent::Document { .. }
                        | AiContent::ToolUse { .. }
                        | AiContent::Thinking { .. }
                        | AiContent::ProviderState { .. } => {}
                        _ => {
                            return Err(AiMessageError::InvalidBlockForRole {
                                role: self.role,
                                block_type: block.type_tag(),
                            });
                        }
                    }
                    if let AiContent::ToolUse { call_id, .. } = block {
                        if call_id.trim().is_empty() {
                            return Err(AiMessageError::MissingCallId);
                        }
                    }
                }
            }
            AiRole::Tool => {
                if self.content.len() != 1 {
                    return Err(AiMessageError::ToolRoleShape);
                }
                let AiContent::ToolResult {
                    call_id, content, ..
                } = &self.content[0]
                else {
                    return Err(AiMessageError::InvalidBlockForRole {
                        role: self.role,
                        block_type: self.content[0].type_tag(),
                    });
                };
                if call_id.trim().is_empty() {
                    return Err(AiMessageError::MissingCallId);
                }
                if content.is_empty() {
                    return Err(AiMessageError::EmptyToolResult);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_units: Option<u64>,
}

impl AiUsage {
    pub fn request_units(request_units: u64) -> Self {
        Self {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            request_units: Some(request_units),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiArtifact {
    pub name: String,
    pub resource: ResourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteAttemptOutcome {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteTraceAttempt {
    pub step: u32,
    pub exact_model: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub outcome: RouteAttemptOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<AiccErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteTrace {
    #[serde(default)]
    pub attempts: Vec<RouteTraceAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiToolCall {
    pub name: String,
    pub args: HashMap<String, Value>,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiResponse {
    pub message: AiMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl Default for AiResponse {
    fn default() -> Self {
        Self {
            message: AiMessage::text(AiRole::Assistant, String::new()),
            usage: None,
            cost: None,
            finish_reason: None,
            provider_task_ref: None,
            extra: None,
        }
    }
}

impl AiResponse {
    pub fn new(message: AiMessage) -> Self {
        Self {
            message,
            ..Self::default()
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new(AiMessage::text(AiRole::Assistant, text))
    }

    pub fn from_parts(
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
        artifacts: Vec<AiArtifact>,
    ) -> Self {
        Self::new(Self::message_from_parts(text, tool_calls, artifacts))
    }

    pub fn message_from_parts(
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
        artifacts: Vec<AiArtifact>,
    ) -> AiMessage {
        let mut content = Vec::new();
        if let Some(text) = text {
            content.push(AiContent::Text { text });
        }
        for call in tool_calls {
            content.push(AiContent::ToolUse {
                call_id: call.call_id,
                name: call.name,
                args: call.args,
            });
        }
        for artifact in artifacts {
            content.push(artifact.into_content());
        }
        if content.is_empty() {
            content.push(AiContent::Text {
                text: String::new(),
            });
        }
        AiMessage::new(AiRole::Assistant, content)
    }

    pub fn text_content(&self) -> String {
        self.message.text_content()
    }

    pub fn tool_calls(&self) -> Vec<AiToolCall> {
        self.message.tool_calls()
    }

    pub fn artifacts(&self) -> Vec<AiArtifact> {
        self.message
            .content
            .iter()
            .enumerate()
            .filter_map(|(idx, block)| match block {
                AiContent::Image { source } => Some(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: source.clone(),
                    mime: resource_ref_mime(source),
                    metadata: None,
                }),
                AiContent::Document { source, title } => Some(AiArtifact {
                    name: title
                        .clone()
                        .unwrap_or_else(|| format!("document_{}", idx + 1)),
                    resource: source.clone(),
                    mime: resource_ref_mime(source),
                    metadata: None,
                }),
                _ => None,
            })
            .collect()
    }

    pub fn validate(&self) -> std::result::Result<(), AiMessageError> {
        if self.message.role != AiRole::Assistant {
            return Err(AiMessageError::InvalidBlockForRole {
                role: self.message.role,
                block_type: "response_message",
            });
        }
        self.message.validate()
    }
}

impl AiArtifact {
    pub fn into_content(self) -> AiContent {
        let is_image = self
            .mime
            .as_deref()
            .map(|mime| mime.starts_with("image/"))
            .unwrap_or(false);
        if is_image {
            AiContent::Image {
                source: self.resource,
            }
        } else {
            AiContent::Document {
                source: self.resource,
                title: Some(self.name),
            }
        }
    }
}

fn resource_ref_mime(source: &ResourceRef) -> Option<String> {
    match source {
        ResourceRef::Url { mime_hint, .. } => mime_hint.clone(),
        ResourceRef::Base64 { mime, .. } => Some(mime.clone()),
        ResourceRef::NamedObject { .. } => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiTaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiMethodStatus {
    Succeeded,
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteResolveRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub api_type: ApiType,
    pub logical_model: String,
    #[serde(default)]
    pub requirements: ModelRequirement,
    #[serde(default, skip_serializing_if = "is_default_model_disable")]
    pub disable: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overlay: Option<AiccRouteOverlay>,
}

impl RouteResolveRequest {
    pub fn new(api_type: ApiType, logical_model: impl Into<String>) -> Self {
        Self {
            request_id: None,
            api_type,
            logical_model: logical_model.into(),
            requirements: ModelRequirement::default(),
            disable: ModelDisable::default(),
            policy: None,
            estimated_input_tokens: None,
            estimated_output_tokens: None,
            session_overlay: None,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RouteResolveRequest: {}", error))
        })?;
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteFallbackAttempt {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub provider_model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteResolveResponse {
    pub selected_exact_model: String,
    pub selected_model_uid: String,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_driver: Option<String>,
    pub origin_model_id: String,
    pub provider_model_id: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_capabilities: Vec<Feature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_capabilities: Vec<Feature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_attempts: Vec<RouteFallbackAttempt>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub route_trace: RouteTrace,
    pub inventory_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LlmChatInvokeRequest {
    pub exact_model: String,
    #[serde(default)]
    pub messages: Vec<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AiToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<LlmResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AiOutputOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LlmChatHelperRequest {
    pub logical_model: String,
    #[serde(default)]
    pub requirements: HelperModelRequirement,
    #[serde(default, skip_serializing_if = "is_default_model_disable")]
    pub disable: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(default)]
    pub messages: Vec<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AiToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<LlmResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AiOutputOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overlay: Option<AiccRouteOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct HelperModelRequirement {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub image_generation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
}

impl From<HelperModelRequirement> for ModelRequirement {
    fn from(value: HelperModelRequirement) -> Self {
        Self {
            streaming: value.streaming,
            tool_call: value.tool_call,
            json_schema: value.json_schema,
            web_search: value.web_search,
            vision: value.vision,
            image_generation: value.image_generation,
            min_context_tokens: value.min_context_tokens,
        }
    }
}

impl LlmChatHelperRequest {
    pub fn new(logical_model: impl Into<String>, messages: Vec<AiMessage>) -> Self {
        Self {
            logical_model: logical_model.into(),
            requirements: HelperModelRequirement::default(),
            disable: ModelDisable::default(),
            policy: None,
            messages,
            tools: Vec::new(),
            response_format: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            seed: None,
            stop: Vec::new(),
            output: None,
            idempotency_key: None,
            task_options: None,
            session_overlay: None,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse LlmChatHelperRequest: {}", error))
        })?;
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

impl LlmChatInvokeRequest {
    pub fn new(exact_model: impl Into<String>, messages: Vec<AiMessage>) -> Self {
        Self {
            exact_model: exact_model.into(),
            messages,
            tools: Vec::new(),
            response_format: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            seed: None,
            stop: Vec::new(),
            output: None,
            idempotency_key: None,
            task_options: None,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse LlmChatInvokeRequest: {}", error))
        })?;
        validate_exact_model_name(&request.exact_model).map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmChatInvokeResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AiToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_trace: Option<RouteTrace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AiccError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextToImageInvokeRequest {
    pub exact_model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AiOutputOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

impl TextToImageInvokeRequest {
    pub fn new(exact_model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            exact_model: exact_model.into(),
            prompt: prompt.into(),
            negative_prompt: None,
            n: None,
            aspect_ratio: None,
            size: None,
            quality: None,
            style: None,
            seed: None,
            output: None,
            idempotency_key: None,
            task_options: None,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TextToImageInvokeRequest: {}",
                error
            ))
        })?;
        validate_exact_model_name(&request.exact_model).map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextToImageHelperRequest {
    pub logical_model: String,
    #[serde(default)]
    pub requirements: HelperModelRequirement,
    #[serde(default, skip_serializing_if = "is_default_model_disable")]
    pub disable: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AiOutputOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overlay: Option<AiccRouteOverlay>,
}

impl TextToImageHelperRequest {
    pub fn new(logical_model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            logical_model: logical_model.into(),
            requirements: HelperModelRequirement::default(),
            disable: ModelDisable::default(),
            policy: None,
            prompt: prompt.into(),
            negative_prompt: None,
            n: None,
            aspect_ratio: None,
            size: None,
            quality: None,
            style: None,
            seed: None,
            output: None,
            idempotency_key: None,
            task_options: None,
            session_overlay: None,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TextToImageHelperRequest: {}",
                error
            ))
        })?;
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextToImageInvokeResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ResourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_states: Vec<AiContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_trace: Option<RouteTrace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AiccError>,
}

macro_rules! impl_request_json {
    ($request:ty) => {
        impl $request {
            pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
                serde_json::from_value(value).map_err(|error| {
                    RPCErrors::ParseRequestError(format!(
                        "Failed to parse {}: {}",
                        stringify!($request),
                        error
                    ))
                })
            }
        }
    };
}

macro_rules! typed_request {
    (
        $name:ident,
        required { $( $required:ident : $required_ty:ty ),* $(,)? },
        optional { $( $optional:ident : $optional_ty:ty ),* $(,)? }
    ) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub exact_model: String,
            $(pub $required: $required_ty,)*
            $(
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $optional: Option<$optional_ty>,
            )*
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub idempotency_key: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub task_options: Option<AiTaskOptions>,
        }

        impl $name {
            pub fn new(exact_model: impl Into<String>, $($required: $required_ty),*) -> Self {
                Self {
                    exact_model: exact_model.into(),
                    $($required,)*
                    $($optional: None,)*
                    idempotency_key: None,
                    task_options: None,
                }
            }
        }

        impl $name {
            pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
                let request: Self = serde_json::from_value(value).map_err(|error| {
                    RPCErrors::ParseRequestError(format!(
                        "Failed to parse {}: {}",
                        stringify!($name),
                        error
                    ))
                })?;
                validate_exact_model_name(&request.exact_model)
                    .map_err(|error| error.to_krpc_error())?;
                Ok(request)
            }
        }
    };
}

macro_rules! typed_response {
    ($name:ident { $( $field:ident : $field_ty:ty ),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        pub struct $name {
            pub task_id: String,
            pub status: AiMethodStatus,
            $(
                #[serde(default, skip_serializing_if = "is_default")]
                pub $field: $field_ty,
            )*
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub usage: Option<AiUsage>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub cost: Option<AiCost>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub finish_reason: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub provider_task_ref: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub route_trace: Option<RouteTrace>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub event_ref: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub error: Option<AiccError>,
        }

        impl $name {
            pub fn new(task_id: impl Into<String>, status: AiMethodStatus) -> Self {
                Self {
                    task_id: task_id.into(),
                    status,
                    $($field: Default::default(),)*
                    usage: None,
                    cost: None,
                    finish_reason: None,
                    provider_task_ref: None,
                    route_trace: None,
                    event_ref: None,
                    error: None,
                }
            }
        }
    };
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiOutputOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmbeddingTextItem {
    Text {
        text: String,
        id: Option<String>,
    },
    Resource {
        resource: ResourceRef,
        id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingMultimodalItem {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ResourceRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingChunking {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingValue {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub embedding: Vec<f32>,
    pub embedding_space_id: String,
}

typed_request!(EmbeddingTextRequest,
    required { items: Vec<EmbeddingTextItem> },
    optional {
        chunking: EmbeddingChunking,
        embedding_space_id: String,
        dimensions: u32,
        normalize: bool,
        prefer_artifact: Value
    }
);
typed_request!(EmbeddingMultimodalRequest,
    required { items: Vec<EmbeddingMultimodalItem> },
    optional { dimensions: u32, normalize: bool }
);
typed_response!(EmbeddingTextResponse {
    data: Vec<EmbeddingValue>,
    data_resource: Option<ResourceRef>
});
typed_response!(EmbeddingMultimodalResponse {
    data: Vec<EmbeddingValue>,
    data_resource: Option<ResourceRef>
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RerankDocument {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RerankResult {
    pub index: usize,
    pub id: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<RerankDocument>,
}

typed_request!(RerankRequest,
    required { query: String, documents: Vec<RerankDocument> },
    optional { n: u32, return_documents: bool }
);
typed_response!(RerankResponse { results: Vec<RerankResult> });

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaskSemantics {
    WhiteAreaIsEditArea,
    BlackAreaIsEditArea,
    AlphaZeroIsEditArea,
}

typed_request!(ImageToImageRequest,
    required { images: Vec<ResourceRef>, prompt: String },
    optional { strength: f64, output: AiOutputOptions }
);
typed_request!(
    ImageInpaintRequest,
    required {
        image: ResourceRef,
        mask: ResourceRef,
        prompt: String
    },
    optional {
        mask_semantics: MaskSemantics,
        output: AiOutputOptions
    }
);
typed_request!(
    ImageUpscaleRequest,
    required { image: ResourceRef },
    optional {
        scale: u32,
        target_width: u32,
        target_height: u32,
        preserve_faces: bool,
        output: AiOutputOptions
    }
);
typed_request!(
    ImageBackgroundRemoveRequest,
    required { image: ResourceRef },
    optional {
        mode: String,
        output: AiOutputOptions
    }
);
typed_response!(ImageToImageResponse {
    images: Vec<ResourceRef>,
    provider_states: Vec<AiContent>
});
typed_response!(ImageInpaintResponse {
    images: Vec<ResourceRef>,
    provider_states: Vec<AiContent>
});
typed_response!(ImageUpscaleResponse { image: Option<ResourceRef> });
typed_response!(ImageBackgroundRemoveResponse { image: Option<ResourceRef> });

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundingBoxFormat {
    Xywh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundingBoxUnit {
    Px,
    Relative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoundingBox {
    pub format: BoundingBoxFormat,
    pub unit: BoundingBoxUnit,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OcrLine {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OcrBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub bbox: BoundingBox,
    #[serde(default)]
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OcrPage {
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub blocks: Vec<OcrBlock>,
}

typed_request!(VisionOcrRequest,
    required { document: ResourceRef },
    optional {
        level: String,
        language_hints: Vec<String>,
        return_layout: bool,
        return_artifacts: Vec<String>
    }
);
typed_response!(VisionOcrResponse {
    text: Option<String>,
    pages: Vec<OcrPage>,
    artifacts: BTreeMap<String, ResourceRef>
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Caption {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

typed_request!(
    VisionCaptionRequest,
    required { image: ResourceRef },
    optional {
        style: String,
        language: String,
        n: u32
    }
);
typed_response!(VisionCaptionResponse { captions: Vec<Caption> });

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_id: Option<String>,
    pub score: f64,
    pub bbox: BoundingBox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoundingBoxSpec {
    pub format: BoundingBoxFormat,
    pub unit: BoundingBoxUnit,
}

typed_request!(VisionDetectRequest,
    required { image: ResourceRef },
    optional { classes: Vec<String>, score_threshold: f64, bbox_spec: BoundingBoxSpec }
);
typed_response!(VisionDetectResponse { detections: Vec<Detection> });

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SegmentationPrompt {
    Box {
        bbox: BoundingBox,
    },
    Point {
        x: f64,
        y: f64,
        label: Option<String>,
    },
    Text {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum AiMask {
    Rle { size: [u32; 2], counts: String },
    Polygon { points: Vec<[f64; 2]> },
    BitmapResource { resource: ResourceRef },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SegmentationMask {
    pub id: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<BoundingBox>,
    pub mask: AiMask,
}

typed_request!(
    VisionSegmentRequest,
    required {
        image: ResourceRef,
        prompt: SegmentationPrompt
    },
    optional {
        mask_format: String,
        return_bitmap_mask: bool
    }
);
typed_response!(VisionSegmentResponse { masks: Vec<SegmentationMask> });

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VoiceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub speaker_similarity_required: bool,
}

typed_request!(
    AudioTextToSpeechRequest,
    required {
        text: String,
        voice: VoiceSpec
    },
    optional {
        speed: f64,
        output: AiOutputOptions
    }
);
typed_response!(AudioTextToSpeechResponse { audio: Option<ResourceRef> });

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AsrSegment {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

typed_request!(AudioSpeechRecognitionRequest,
    required { audio: ResourceRef },
    optional {
        language: String,
        timestamps: String,
        diarization: bool,
        output_formats: Vec<String>
    }
);
typed_response!(AudioSpeechRecognitionResponse {
    text: Option<String>,
    segments: Vec<AsrSegment>,
    artifacts: BTreeMap<String, ResourceRef>,
    diagnostic: Option<Value>
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MusicSection {
    pub name: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MusicStructure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<String>,
    #[serde(default)]
    pub sections: Vec<MusicSection>,
}

typed_request!(
    AudioMusicRequest,
    required { prompt: String },
    optional {
        duration_seconds: f64,
        instrumental: bool,
        lyrics: String,
        seed: u64,
        output: AiOutputOptions
    }
);
typed_response!(AudioMusicResponse {
    audio: Option<ResourceRef>,
    structure: Option<MusicStructure>
});

typed_request!(
    AudioEnhanceRequest,
    required {
        audio: ResourceRef,
        task: String
    },
    optional {
        strength: f64,
        return_stems: bool
    }
);
typed_response!(AudioEnhanceResponse {
    audio: Option<ResourceRef>,
    stems: Vec<ResourceRef>
});

typed_request!(
    VideoTextToVideoRequest,
    required { prompt: String },
    optional {
        duration_seconds: f64,
        aspect_ratio: String,
        resolution: String,
        generate_audio: bool,
        seed: u64,
        output: AiOutputOptions
    }
);
typed_request!(
    VideoImageToVideoRequest,
    required {
        image: ResourceRef,
        prompt: String
    },
    optional {
        duration_seconds: f64,
        aspect_ratio: String,
        resolution: String
    }
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

typed_request!(
    VideoToVideoRequest,
    required {
        video: ResourceRef,
        prompt: String
    },
    optional {
        preserve_motion: bool,
        time_range: TimeRange
    }
);
typed_request!(
    VideoExtendRequest,
    required {
        video: ResourceRef,
        prompt: String
    },
    optional {
        continuation_handle: String,
        duration_seconds: f64,
        resolution: String
    }
);
typed_request!(
    VideoUpscaleRequest,
    required {
        video: ResourceRef,
        target_resolution: String
    },
    optional {
        denoise: bool,
        sharpen: f64,
        output: AiOutputOptions
    }
);
typed_response!(VideoTextToVideoResponse { video: Option<ResourceRef> });
typed_response!(VideoImageToVideoResponse { video: Option<ResourceRef> });
typed_response!(VideoToVideoResponse { video: Option<ResourceRef> });
typed_response!(VideoExtendResponse { video: Option<ResourceRef> });
typed_response!(VideoUpscaleResponse { video: Option<ResourceRef> });

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComputerEnvironment {
    pub environment_id: String,
    pub session_id: String,
    pub screenshot: ResourceRef,
    pub viewport: Viewport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerAction {
    Screenshot,
    LeftClick { x: f64, y: f64 },
    RightClick { x: f64, y: f64 },
    Type { text: String },
    Key { key: String },
    Scroll { delta_x: f64, delta_y: f64 },
    Wait { duration_ms: u64 },
}

typed_request!(ComputerUseRequest,
    required {
        task: String,
        environment: ComputerEnvironment,
        allowed_actions: Vec<String>
    },
    optional {}
);
typed_response!(ComputerUseResponse {
    actions: Vec<ComputerAction>,
    requires_next_observation: bool
});

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceReloadSettingsRequest {}

impl_request_json!(ServiceReloadSettingsRequest);

impl ServiceReloadSettingsRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceReloadSettingsResponse {
    pub ok: bool,
    pub settings_revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListModelsRequest {}

impl_request_json!(ListModelsRequest);

impl ListModelsRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogRequest {}

impl_request_json!(ProviderCatalogRequest);

impl ProviderCatalogRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogEntry {
    pub provider_profile_id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocol_adapter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ui_hints: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogResponse {
    pub catalog_revision: u64,
    #[serde(default)]
    pub providers: Vec<ProviderCatalogEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolAdapterListRequest {}

impl_request_json!(ProtocolAdapterListRequest);

impl ProtocolAdapterListRequest {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolAdapterStatus {
    Stable,
    Preview,
    Deprecated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolExecutionMode {
    Immediate,
    Stream,
    NativeTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolAdapterOperation {
    pub operation_id: String,
    #[serde(default)]
    pub api_types: Vec<ApiType>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub supported_features: Vec<String>,
    #[serde(default)]
    pub execution_modes: Vec<ProtocolExecutionMode>,
    pub supports_cancel: bool,
    pub supports_webhook: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolAdapterView {
    pub protocol_family_id: String,
    pub protocol_adapter_id: String,
    pub interface_generation: String,
    pub status: ProtocolAdapterStatus,
    pub probe_priority: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_adapter_id: Option<String>,
    #[serde(default)]
    pub operations: Vec<ProtocolAdapterOperation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolAdapterListResponse {
    #[serde(default)]
    pub adapters: Vec<ProtocolAdapterView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderValidateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_name: Option<String>,
    pub provider_type: String,
    pub provider_profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_family_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_adapter_id: Option<String>,
    pub base_url: String,
    pub credentials: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_rules: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync_models: Option<bool>,
}

impl_request_json!(ProviderValidateRequest);

impl ProviderValidateRequest {
    pub fn new(
        provider_type: impl Into<String>,
        provider_profile_id: impl Into<String>,
        base_url: impl Into<String>,
        credentials: Value,
    ) -> Self {
        Self {
            provider_instance_name: None,
            provider_type: provider_type.into(),
            provider_profile_id: provider_profile_id.into(),
            protocol_family_id: None,
            protocol_adapter_id: None,
            base_url: base_url.into(),
            credentials,
            region: None,
            workspace: None,
            account: None,
            provider_rules_id: None,
            auth: None,
            discovery: None,
            instance_rules: None,
            timeout_ms: None,
            auto_sync_models: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderValidationErrorKind {
    Configuration,
    BaseUrl,
    Authentication,
    Protocol,
    Models,
    Balance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderValidationErrorDetail {
    pub kind: ProviderValidationErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderValidateResponse {
    pub base_url_reachable: bool,
    pub auth_valid: bool,
    #[serde(default)]
    pub models_discovered: Vec<String>,
    pub balance_available: bool,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub error_details: Vec<ProviderValidationErrorDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_protocol_adapter_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderAddRequest {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_family_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_adapter_id: Option<String>,
    pub base_url: String,
    pub credentials: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_rules: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync_models: Option<bool>,
}

impl_request_json!(ProviderAddRequest);

impl ProviderAddRequest {
    pub fn new(
        provider_instance_name: impl Into<String>,
        provider_type: impl Into<String>,
        provider_profile_id: impl Into<String>,
        base_url: impl Into<String>,
        credentials: Value,
    ) -> Self {
        Self {
            provider_instance_name: provider_instance_name.into(),
            provider_type: provider_type.into(),
            provider_profile_id: provider_profile_id.into(),
            protocol_family_id: None,
            protocol_adapter_id: None,
            base_url: base_url.into(),
            credentials,
            region: None,
            workspace: None,
            account: None,
            provider_rules_id: None,
            auth: None,
            discovery: None,
            instance_rules: None,
            timeout_ms: None,
            auto_sync_models: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderReloadResult {
    pub ok: bool,
    pub providers_registered: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderAddResponse {
    pub ok: bool,
    pub provider_instance_name: String,
    pub settings_revision: u64,
    pub reload: ProviderReloadResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderDeleteRequest {
    pub provider_instance_name: String,
}

impl_request_json!(ProviderDeleteRequest);

impl ProviderDeleteRequest {
    pub fn new(provider_instance_name: impl Into<String>) -> Self {
        Self {
            provider_instance_name: provider_instance_name.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderDeleteResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reload: Option<ProviderReloadResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRefreshModelsRequest {
    pub provider_instance_name: String,
}

impl_request_json!(ProviderRefreshModelsRequest);

impl ProviderRefreshModelsRequest {
    pub fn new(provider_instance_name: impl Into<String>) -> Self {
        Self {
            provider_instance_name: provider_instance_name.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRefreshModelsResponse {
    pub ok: bool,
    pub provider_instance_name: String,
    pub inventory_revision: String,
}

impl_request_json!(QueryUsageRequest);
impl_request_json!(QueryRouteTraceRequest);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaQueryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<Capability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

impl_request_json!(QuotaQueryRequest);

impl QuotaQueryRequest {
    pub fn new(capability: Option<Capability>, method: Option<String>) -> Self {
        Self { capability, method }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaState {
    Normal,
    NearLimit,
    Exhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaView {
    pub state: QuotaState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_request_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaQueryResponse {
    pub quota: QuotaView,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

impl_request_json!(ProviderListRequest);

impl ProviderListRequest {
    pub fn new(method: Option<String>) -> Self {
        Self { method }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderListResponse {
    #[serde(default)]
    pub providers: Vec<Value>,
    pub inventory_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderHealthRequest {
    pub exact_model: String,
}

impl ProviderHealthRequest {
    pub fn new(exact_model: impl Into<String>) -> Self {
        Self {
            exact_model: exact_model.into(),
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ProviderHealthRequest: {}",
                error
            ))
        })?;
        validate_exact_model_name(&request.exact_model).map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderHealthResponse {
    pub health: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderUpdateRequest {
    pub provider_instance_name: String,
    pub settings_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_adapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_rules: Option<Value>,
}

impl_request_json!(ProviderUpdateRequest);

impl ProviderUpdateRequest {
    pub fn new(provider_instance_name: impl Into<String>, settings_revision: u64) -> Self {
        Self {
            provider_instance_name: provider_instance_name.into(),
            settings_revision,
            enabled: None,
            base_url: None,
            credential: None,
            provider_profile_id: None,
            protocol_adapter_id: None,
            discovery: None,
            instance_rules: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderUpdateResponse {
    pub ok: bool,
    pub settings_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiccCall {
    RouteResolve(RouteResolveRequest),
    ChatCompletionsCreate(LlmChatInvokeRequest),
    ImagesGenerate(TextToImageInvokeRequest),
    HelperLlmChat(LlmChatHelperRequest),
    HelperTextToImage(TextToImageHelperRequest),
    EmbeddingText(EmbeddingTextRequest),
    EmbeddingMultimodal(EmbeddingMultimodalRequest),
    Rerank(RerankRequest),
    ImageToImage(ImageToImageRequest),
    ImageInpaint(ImageInpaintRequest),
    ImageUpscale(ImageUpscaleRequest),
    ImageBackgroundRemove(ImageBackgroundRemoveRequest),
    VisionOcr(VisionOcrRequest),
    VisionCaption(VisionCaptionRequest),
    VisionDetect(VisionDetectRequest),
    VisionSegment(VisionSegmentRequest),
    AudioTextToSpeech(AudioTextToSpeechRequest),
    AudioSpeechRecognition(AudioSpeechRecognitionRequest),
    AudioMusic(AudioMusicRequest),
    AudioEnhance(AudioEnhanceRequest),
    VideoTextToVideo(VideoTextToVideoRequest),
    VideoImageToVideo(VideoImageToVideoRequest),
    VideoToVideo(VideoToVideoRequest),
    VideoExtend(VideoExtendRequest),
    VideoUpscale(VideoUpscaleRequest),
    ComputerUse(ComputerUseRequest),
}

impl AiccCall {
    pub fn method(&self) -> &'static str {
        match self {
            Self::RouteResolve(_) => ai_methods::ROUTE_RESOLVE,
            Self::ChatCompletionsCreate(_) => ai_methods::CHAT_COMPLETIONS_CREATE,
            Self::ImagesGenerate(_) => ai_methods::IMAGES_GENERATE,
            Self::HelperLlmChat(_) => ai_methods::HELPER_LLM_CHAT,
            Self::HelperTextToImage(_) => ai_methods::HELPER_TEXT_TO_IMAGE,
            Self::EmbeddingText(_) => ai_methods::EMBEDDING_TEXT,
            Self::EmbeddingMultimodal(_) => ai_methods::EMBEDDING_MULTIMODAL,
            Self::Rerank(_) => ai_methods::RERANK,
            Self::ImageToImage(_) => ai_methods::IMAGE_IMG2IMG,
            Self::ImageInpaint(_) => ai_methods::IMAGE_INPAINT,
            Self::ImageUpscale(_) => ai_methods::IMAGE_UPSCALE,
            Self::ImageBackgroundRemove(_) => ai_methods::IMAGE_BG_REMOVE,
            Self::VisionOcr(_) => ai_methods::VISION_OCR,
            Self::VisionCaption(_) => ai_methods::VISION_CAPTION,
            Self::VisionDetect(_) => ai_methods::VISION_DETECT,
            Self::VisionSegment(_) => ai_methods::VISION_SEGMENT,
            Self::AudioTextToSpeech(_) => ai_methods::AUDIO_TTS,
            Self::AudioSpeechRecognition(_) => ai_methods::AUDIO_ASR,
            Self::AudioMusic(_) => ai_methods::AUDIO_MUSIC,
            Self::AudioEnhance(_) => ai_methods::AUDIO_ENHANCE,
            Self::VideoTextToVideo(_) => ai_methods::VIDEO_TXT2VIDEO,
            Self::VideoImageToVideo(_) => ai_methods::VIDEO_IMG2VIDEO,
            Self::VideoToVideo(_) => ai_methods::VIDEO_VIDEO2VIDEO,
            Self::VideoExtend(_) => ai_methods::VIDEO_EXTEND,
            Self::VideoUpscale(_) => ai_methods::VIDEO_UPSCALE,
            Self::ComputerUse(_) => ai_methods::AGENT_COMPUTER_USE,
        }
    }

    pub fn from_method_and_params(
        method: &str,
        params: Value,
    ) -> std::result::Result<Self, RPCErrors> {
        let parse = |error: serde_json::Error| {
            RPCErrors::ParseRequestError(format!("invalid {method} request: {error}"))
        };
        match method {
            ai_methods::ROUTE_RESOLVE => Ok(Self::RouteResolve(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::CHAT_COMPLETIONS_CREATE => Ok(Self::ChatCompletionsCreate(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::IMAGES_GENERATE => Ok(Self::ImagesGenerate(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::HELPER_LLM_CHAT => Ok(Self::HelperLlmChat(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::HELPER_TEXT_TO_IMAGE => Ok(Self::HelperTextToImage(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::EMBEDDING_TEXT => Ok(Self::EmbeddingText(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::EMBEDDING_MULTIMODAL => Ok(Self::EmbeddingMultimodal(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::RERANK => Ok(Self::Rerank(serde_json::from_value(params).map_err(parse)?)),
            ai_methods::IMAGE_IMG2IMG => Ok(Self::ImageToImage(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::IMAGE_INPAINT => Ok(Self::ImageInpaint(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::IMAGE_UPSCALE => Ok(Self::ImageUpscale(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::IMAGE_BG_REMOVE => Ok(Self::ImageBackgroundRemove(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VISION_OCR => Ok(Self::VisionOcr(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VISION_CAPTION => Ok(Self::VisionCaption(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VISION_DETECT => Ok(Self::VisionDetect(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VISION_SEGMENT => Ok(Self::VisionSegment(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::AUDIO_TTS => Ok(Self::AudioTextToSpeech(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::AUDIO_ASR => Ok(Self::AudioSpeechRecognition(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::AUDIO_MUSIC => Ok(Self::AudioMusic(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::AUDIO_ENHANCE => Ok(Self::AudioEnhance(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VIDEO_TXT2VIDEO => Ok(Self::VideoTextToVideo(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VIDEO_IMG2VIDEO => Ok(Self::VideoImageToVideo(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VIDEO_VIDEO2VIDEO => Ok(Self::VideoToVideo(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VIDEO_EXTEND => Ok(Self::VideoExtend(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::VIDEO_UPSCALE => Ok(Self::VideoUpscale(
                serde_json::from_value(params).map_err(parse)?,
            )),
            ai_methods::AGENT_COMPUTER_USE => Ok(Self::ComputerUse(
                serde_json::from_value(params).map_err(parse)?,
            )),
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub task_id: String,
}

impl CancelRequest {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse CancelRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelResponse {
    pub task_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataUpdateGetReq {}

impl DriverMetadataUpdateGetReq {
    pub fn new() -> Self {
        Self {}
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse DriverMetadataUpdateGetReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataUpdateSetReq {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
}

impl DriverMetadataUpdateSetReq {
    pub fn new(enabled: bool, source_url: Option<String>, interval_secs: Option<u64>) -> Self {
        Self {
            enabled,
            source_url,
            interval_secs,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse DriverMetadataUpdateSetReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverMetadataUpdateStatus {
    Disabled,
    Idle,
    Updating,
    Healthy,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataProviderStatus {
    pub provider_instance_name: String,
    pub metadata_applied_seq: u64,
}

impl DriverMetadataProviderStatus {
    pub fn new(provider_instance_name: impl Into<String>, metadata_applied_seq: u64) -> Self {
        Self {
            provider_instance_name: provider_instance_name.into(),
            metadata_applied_seq,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataUpdateView {
    pub enabled: bool,
    #[serde(default)]
    pub source_url: Option<String>,
    pub source_configured: bool,
    pub interval_secs: u64,
    pub metadata_target_seq: u64,
    pub providers: Vec<DriverMetadataProviderStatus>,
    pub status: DriverMetadataUpdateStatus,
    #[serde(default)]
    pub active_revision: Option<u64>,
    #[serde(default)]
    pub last_attempt_at_ms: Option<u64>,
    #[serde(default)]
    pub last_success_at_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataRuntimeApply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_scheduled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataUpdateSetResponse {
    pub ok: bool,
    pub settings_revision: u64,
    pub settings: DriverMetadataUpdateView,
    pub runtime_apply: DriverMetadataRuntimeApply,
}

impl CancelResponse {
    pub fn new(task_id: String, accepted: bool) -> Self {
        Self { task_id, accepted }
    }
}

pub enum AiccClient {
    InProcess(Box<dyn AiccHandler>),
    KRPC(Box<kRPC>),
}

macro_rules! client_typed_method {
    ($method_fn:ident, $handler_fn:ident, $method:expr, $request:ty, $response:ty) => {
        pub async fn $method_fn(
            &self,
            request: $request,
        ) -> std::result::Result<$response, RPCErrors> {
            match self {
                Self::InProcess(handler) => {
                    let ctx = RPCContext::default();
                    handler.$handler_fn(request, ctx).await
                }
                Self::KRPC(client) => {
                    let request = serde_json::to_value(request).map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Failed to serialize {}: {}",
                            stringify!($request),
                            error
                        ))
                    })?;
                    let result = client.call($method, request).await?;
                    serde_json::from_value(result).map_err(|error| {
                        RPCErrors::ParserResponseError(format!(
                            "Failed to parse {}: {}",
                            stringify!($response),
                            error
                        ))
                    })
                }
            }
        }
    };
}

macro_rules! client_inference_method {
    ($method_fn:ident, $handler_fn:ident, $method:expr, $request:ty, $response:ty) => {
        pub async fn $method_fn(
            &self,
            request: $request,
        ) -> std::result::Result<$response, RPCErrors> {
            validate_exact_model_name(&request.exact_model)
                .map_err(|error| error.to_krpc_error())?;
            match self {
                Self::InProcess(handler) => {
                    let ctx = RPCContext::default();
                    handler.$handler_fn(request, ctx).await
                }
                Self::KRPC(client) => {
                    let request = serde_json::to_value(request).map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Failed to serialize {}: {}",
                            stringify!($request),
                            error
                        ))
                    })?;
                    let result = client.call($method, request).await?;
                    serde_json::from_value(result).map_err(|error| {
                        RPCErrors::ParserResponseError(format!(
                            "Failed to parse {}: {}",
                            stringify!($response),
                            error
                        ))
                    })
                }
            }
        }
    };
}

impl AiccClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn AiccHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn invoke(&self, call: AiccCall) -> std::result::Result<Value, RPCErrors> {
        let result = match call {
            AiccCall::RouteResolve(request) => {
                serde_json::to_value(self.route_resolve(request).await?)
            }
            AiccCall::ChatCompletionsCreate(request) => {
                serde_json::to_value(self.chat_completions_create(request).await?)
            }
            AiccCall::ImagesGenerate(request) => {
                serde_json::to_value(self.images_generate(request).await?)
            }
            AiccCall::HelperLlmChat(request) => {
                serde_json::to_value(self.helper_llm_chat(request).await?)
            }
            AiccCall::HelperTextToImage(request) => {
                serde_json::to_value(self.helper_text_to_image(request).await?)
            }
            AiccCall::EmbeddingText(request) => {
                serde_json::to_value(self.embedding_text(request).await?)
            }
            AiccCall::EmbeddingMultimodal(request) => {
                serde_json::to_value(self.embedding_multimodal(request).await?)
            }
            AiccCall::Rerank(request) => serde_json::to_value(self.rerank(request).await?),
            AiccCall::ImageToImage(request) => {
                serde_json::to_value(self.image_to_image(request).await?)
            }
            AiccCall::ImageInpaint(request) => {
                serde_json::to_value(self.image_inpaint(request).await?)
            }
            AiccCall::ImageUpscale(request) => {
                serde_json::to_value(self.image_upscale(request).await?)
            }
            AiccCall::ImageBackgroundRemove(request) => {
                serde_json::to_value(self.image_background_remove(request).await?)
            }
            AiccCall::VisionOcr(request) => serde_json::to_value(self.vision_ocr(request).await?),
            AiccCall::VisionCaption(request) => {
                serde_json::to_value(self.vision_caption(request).await?)
            }
            AiccCall::VisionDetect(request) => {
                serde_json::to_value(self.vision_detect(request).await?)
            }
            AiccCall::VisionSegment(request) => {
                serde_json::to_value(self.vision_segment(request).await?)
            }
            AiccCall::AudioTextToSpeech(request) => {
                serde_json::to_value(self.audio_text_to_speech(request).await?)
            }
            AiccCall::AudioSpeechRecognition(request) => {
                serde_json::to_value(self.audio_speech_recognition(request).await?)
            }
            AiccCall::AudioMusic(request) => serde_json::to_value(self.audio_music(request).await?),
            AiccCall::AudioEnhance(request) => {
                serde_json::to_value(self.audio_enhance(request).await?)
            }
            AiccCall::VideoTextToVideo(request) => {
                serde_json::to_value(self.video_text_to_video(request).await?)
            }
            AiccCall::VideoImageToVideo(request) => {
                serde_json::to_value(self.video_image_to_video(request).await?)
            }
            AiccCall::VideoToVideo(request) => {
                serde_json::to_value(self.video_to_video(request).await?)
            }
            AiccCall::VideoExtend(request) => {
                serde_json::to_value(self.video_extend(request).await?)
            }
            AiccCall::VideoUpscale(request) => {
                serde_json::to_value(self.video_upscale(request).await?)
            }
            AiccCall::ComputerUse(request) => {
                serde_json::to_value(self.computer_use(request).await?)
            }
        };
        result.map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to serialize AICC response: {error}"))
        })
    }

    pub async fn route_resolve(
        &self,
        request: RouteResolveRequest,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_route_resolve(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize RouteResolveRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::ROUTE_RESOLVE, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse route.resolve response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn chat_completions_create(
        &self,
        request: LlmChatInvokeRequest,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        validate_exact_model_name(&request.exact_model).map_err(|error| error.to_krpc_error())?;
        request
            .messages
            .iter()
            .try_for_each(AiMessage::validate)
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}")))?;
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_chat_completions_create(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize LlmChatInvokeRequest: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::CHAT_COMPLETIONS_CREATE, req_json)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse chat.completions.create response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn images_generate(
        &self,
        request: TextToImageInvokeRequest,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        validate_exact_model_name(&request.exact_model).map_err(|error| error.to_krpc_error())?;
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_images_generate(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize TextToImageInvokeRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::IMAGES_GENERATE, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse images.generate response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn helper_llm_chat(
        &self,
        request: LlmChatHelperRequest,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        request
            .messages
            .iter()
            .try_for_each(AiMessage::validate)
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}")))?;

        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_helper_llm_chat(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize LlmChatHelperRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::HELPER_LLM_CHAT, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse helper.llm_chat response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn helper_text_to_image(
        &self,
        request: TextToImageHelperRequest,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        validate_logical_model_name(&request.logical_model)
            .map_err(|error| error.to_krpc_error())?;
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_helper_text_to_image(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize TextToImageHelperRequest: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::HELPER_TEXT_TO_IMAGE, req_json)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse helper.text_to_image response: {}",
                        error
                    ))
                })
            }
        }
    }

    client_inference_method!(
        embedding_text,
        handle_embedding_text,
        ai_methods::EMBEDDING_TEXT,
        EmbeddingTextRequest,
        EmbeddingTextResponse
    );
    client_inference_method!(
        embedding_multimodal,
        handle_embedding_multimodal,
        ai_methods::EMBEDDING_MULTIMODAL,
        EmbeddingMultimodalRequest,
        EmbeddingMultimodalResponse
    );
    client_inference_method!(
        rerank,
        handle_rerank,
        ai_methods::RERANK,
        RerankRequest,
        RerankResponse
    );
    client_inference_method!(
        image_to_image,
        handle_image_to_image,
        ai_methods::IMAGE_IMG2IMG,
        ImageToImageRequest,
        ImageToImageResponse
    );
    client_inference_method!(
        image_inpaint,
        handle_image_inpaint,
        ai_methods::IMAGE_INPAINT,
        ImageInpaintRequest,
        ImageInpaintResponse
    );
    client_inference_method!(
        image_upscale,
        handle_image_upscale,
        ai_methods::IMAGE_UPSCALE,
        ImageUpscaleRequest,
        ImageUpscaleResponse
    );
    client_inference_method!(
        image_background_remove,
        handle_image_background_remove,
        ai_methods::IMAGE_BG_REMOVE,
        ImageBackgroundRemoveRequest,
        ImageBackgroundRemoveResponse
    );
    client_inference_method!(
        vision_ocr,
        handle_vision_ocr,
        ai_methods::VISION_OCR,
        VisionOcrRequest,
        VisionOcrResponse
    );
    client_inference_method!(
        vision_caption,
        handle_vision_caption,
        ai_methods::VISION_CAPTION,
        VisionCaptionRequest,
        VisionCaptionResponse
    );
    client_inference_method!(
        vision_detect,
        handle_vision_detect,
        ai_methods::VISION_DETECT,
        VisionDetectRequest,
        VisionDetectResponse
    );
    client_inference_method!(
        vision_segment,
        handle_vision_segment,
        ai_methods::VISION_SEGMENT,
        VisionSegmentRequest,
        VisionSegmentResponse
    );
    client_inference_method!(
        audio_text_to_speech,
        handle_audio_text_to_speech,
        ai_methods::AUDIO_TTS,
        AudioTextToSpeechRequest,
        AudioTextToSpeechResponse
    );
    client_inference_method!(
        audio_speech_recognition,
        handle_audio_speech_recognition,
        ai_methods::AUDIO_ASR,
        AudioSpeechRecognitionRequest,
        AudioSpeechRecognitionResponse
    );
    client_inference_method!(
        audio_music,
        handle_audio_music,
        ai_methods::AUDIO_MUSIC,
        AudioMusicRequest,
        AudioMusicResponse
    );
    client_inference_method!(
        audio_enhance,
        handle_audio_enhance,
        ai_methods::AUDIO_ENHANCE,
        AudioEnhanceRequest,
        AudioEnhanceResponse
    );
    client_inference_method!(
        video_text_to_video,
        handle_video_text_to_video,
        ai_methods::VIDEO_TXT2VIDEO,
        VideoTextToVideoRequest,
        VideoTextToVideoResponse
    );
    client_inference_method!(
        video_image_to_video,
        handle_video_image_to_video,
        ai_methods::VIDEO_IMG2VIDEO,
        VideoImageToVideoRequest,
        VideoImageToVideoResponse
    );
    client_inference_method!(
        video_to_video,
        handle_video_to_video,
        ai_methods::VIDEO_VIDEO2VIDEO,
        VideoToVideoRequest,
        VideoToVideoResponse
    );
    client_inference_method!(
        video_extend,
        handle_video_extend,
        ai_methods::VIDEO_EXTEND,
        VideoExtendRequest,
        VideoExtendResponse
    );
    client_inference_method!(
        video_upscale,
        handle_video_upscale,
        ai_methods::VIDEO_UPSCALE,
        VideoUpscaleRequest,
        VideoUpscaleResponse
    );
    client_inference_method!(
        computer_use,
        handle_computer_use,
        ai_methods::AGENT_COMPUTER_USE,
        ComputerUseRequest,
        ComputerUseResponse
    );
    client_typed_method!(
        reload_settings,
        handle_reload_settings,
        ai_methods::SERVICE_RELOAD_SETTINGS,
        ServiceReloadSettingsRequest,
        ServiceReloadSettingsResponse
    );
    client_typed_method!(
        query_quota,
        handle_query_quota,
        ai_methods::QUOTA_QUERY,
        QuotaQueryRequest,
        QuotaQueryResponse
    );
    client_typed_method!(
        query_usage,
        handle_query_usage,
        ai_methods::USAGE_QUERY,
        QueryUsageRequest,
        QueryUsageResponse
    );
    client_typed_method!(
        query_trace,
        handle_query_trace,
        ai_methods::TRACE_QUERY,
        QueryRouteTraceRequest,
        QueryRouteTraceResponse
    );
    client_typed_method!(
        provider_catalog,
        handle_provider_catalog,
        ai_methods::PROVIDER_CATALOG,
        ProviderCatalogRequest,
        ProviderCatalogResponse
    );
    client_typed_method!(
        list_protocol_adapters,
        handle_list_protocol_adapters,
        ai_methods::PROTOCOL_ADAPTER_LIST,
        ProtocolAdapterListRequest,
        ProtocolAdapterListResponse
    );
    client_typed_method!(
        validate_provider,
        handle_validate_provider,
        ai_methods::PROVIDER_VALIDATE,
        ProviderValidateRequest,
        ProviderValidateResponse
    );
    client_typed_method!(
        add_provider,
        handle_add_provider,
        ai_methods::PROVIDER_ADD,
        ProviderAddRequest,
        ProviderAddResponse
    );
    client_typed_method!(
        list_providers,
        handle_list_providers,
        ai_methods::PROVIDER_LIST,
        ProviderListRequest,
        ProviderListResponse
    );
    client_inference_method!(
        provider_health,
        handle_provider_health,
        ai_methods::PROVIDER_HEALTH,
        ProviderHealthRequest,
        ProviderHealthResponse
    );
    client_typed_method!(
        update_provider,
        handle_update_provider,
        ai_methods::PROVIDER_UPDATE,
        ProviderUpdateRequest,
        ProviderUpdateResponse
    );
    client_typed_method!(
        delete_provider,
        handle_delete_provider,
        ai_methods::PROVIDER_DELETE,
        ProviderDeleteRequest,
        ProviderDeleteResponse
    );
    client_typed_method!(
        refresh_provider_models,
        handle_refresh_provider_models,
        ai_methods::PROVIDER_REFRESH_MODELS,
        ProviderRefreshModelsRequest,
        ProviderRefreshModelsResponse
    );

    pub async fn cancel(&self, task_id: &str) -> std::result::Result<CancelResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_cancel(task_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = CancelRequest::new(task_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to serialize CancelRequest: {}", error))
                })?;
                let result = client.call(ai_methods::CANCEL, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse cancel response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_models(&self) -> std::result::Result<Value, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_list_models(ListModelsRequest::new(), RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let request = serde_json::to_value(ListModelsRequest::new()).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ListModelsRequest: {}",
                        error
                    ))
                })?;
                client.call(ai_methods::MODELS_LIST, request).await
            }
        }
    }

    pub async fn get_driver_metadata_update(
        &self,
    ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_driver_metadata_update_get(ctx).await
            }
            Self::KRPC(client) => {
                let request = DriverMetadataUpdateGetReq::new();
                let request = serde_json::to_value(request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize DriverMetadataUpdateGetReq: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::DRIVER_METADATA_UPDATE_GET, request)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse driver metadata update view: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn set_driver_metadata_update(
        &self,
        request: DriverMetadataUpdateSetReq,
    ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_driver_metadata_update_set(request, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let request = serde_json::to_value(request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize DriverMetadataUpdateSetReq: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::DRIVER_METADATA_UPDATE_SET, request)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse driver metadata update set response: {}",
                        error
                    ))
                })
            }
        }
    }
}

#[async_trait]
pub trait AiccHandler: Send + Sync {
    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors>;

    async fn handle_route_resolve(
        &self,
        _request: RouteResolveRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::ROUTE_RESOLVE.to_string(),
        ))
    }

    async fn handle_chat_completions_create(
        &self,
        _request: LlmChatInvokeRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
        ))
    }

    async fn handle_images_generate(
        &self,
        _request: TextToImageInvokeRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGES_GENERATE.to_string(),
        ))
    }

    async fn handle_helper_llm_chat(
        &self,
        _request: LlmChatHelperRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::HELPER_LLM_CHAT.to_string(),
        ))
    }

    async fn handle_helper_text_to_image(
        &self,
        _request: TextToImageHelperRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::HELPER_TEXT_TO_IMAGE.to_string(),
        ))
    }

    async fn handle_embedding_text(
        &self,
        _request: EmbeddingTextRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<EmbeddingTextResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::EMBEDDING_TEXT.to_string(),
        ))
    }

    async fn handle_embedding_multimodal(
        &self,
        _request: EmbeddingMultimodalRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<EmbeddingMultimodalResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::EMBEDDING_MULTIMODAL.to_string(),
        ))
    }

    async fn handle_rerank(
        &self,
        _request: RerankRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<RerankResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(ai_methods::RERANK.to_string()))
    }

    async fn handle_image_to_image(
        &self,
        _request: ImageToImageRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ImageToImageResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGE_IMG2IMG.to_string(),
        ))
    }

    async fn handle_image_inpaint(
        &self,
        _request: ImageInpaintRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ImageInpaintResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGE_INPAINT.to_string(),
        ))
    }

    async fn handle_image_upscale(
        &self,
        _request: ImageUpscaleRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ImageUpscaleResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGE_UPSCALE.to_string(),
        ))
    }

    async fn handle_image_background_remove(
        &self,
        _request: ImageBackgroundRemoveRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ImageBackgroundRemoveResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGE_BG_REMOVE.to_string(),
        ))
    }

    async fn handle_vision_ocr(
        &self,
        _request: VisionOcrRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VisionOcrResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(ai_methods::VISION_OCR.to_string()))
    }

    async fn handle_vision_caption(
        &self,
        _request: VisionCaptionRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VisionCaptionResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VISION_CAPTION.to_string(),
        ))
    }

    async fn handle_vision_detect(
        &self,
        _request: VisionDetectRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VisionDetectResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VISION_DETECT.to_string(),
        ))
    }

    async fn handle_vision_segment(
        &self,
        _request: VisionSegmentRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VisionSegmentResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VISION_SEGMENT.to_string(),
        ))
    }

    async fn handle_audio_text_to_speech(
        &self,
        _request: AudioTextToSpeechRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AudioTextToSpeechResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(ai_methods::AUDIO_TTS.to_string()))
    }

    async fn handle_audio_speech_recognition(
        &self,
        _request: AudioSpeechRecognitionRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AudioSpeechRecognitionResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(ai_methods::AUDIO_ASR.to_string()))
    }

    async fn handle_audio_music(
        &self,
        _request: AudioMusicRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AudioMusicResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::AUDIO_MUSIC.to_string(),
        ))
    }

    async fn handle_audio_enhance(
        &self,
        _request: AudioEnhanceRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AudioEnhanceResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::AUDIO_ENHANCE.to_string(),
        ))
    }

    async fn handle_video_text_to_video(
        &self,
        _request: VideoTextToVideoRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VideoTextToVideoResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VIDEO_TXT2VIDEO.to_string(),
        ))
    }

    async fn handle_video_image_to_video(
        &self,
        _request: VideoImageToVideoRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VideoImageToVideoResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VIDEO_IMG2VIDEO.to_string(),
        ))
    }

    async fn handle_video_to_video(
        &self,
        _request: VideoToVideoRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VideoToVideoResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VIDEO_VIDEO2VIDEO.to_string(),
        ))
    }

    async fn handle_video_extend(
        &self,
        _request: VideoExtendRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VideoExtendResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VIDEO_EXTEND.to_string(),
        ))
    }

    async fn handle_video_upscale(
        &self,
        _request: VideoUpscaleRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<VideoUpscaleResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::VIDEO_UPSCALE.to_string(),
        ))
    }

    async fn handle_computer_use(
        &self,
        _request: ComputerUseRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ComputerUseResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::AGENT_COMPUTER_USE.to_string(),
        ))
    }

    async fn handle_reload_settings(
        &self,
        _request: ServiceReloadSettingsRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ServiceReloadSettingsResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::SERVICE_RELOAD_SETTINGS.to_string(),
        ))
    }

    async fn handle_query_quota(
        &self,
        _request: QuotaQueryRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<QuotaQueryResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::QUOTA_QUERY.to_string(),
        ))
    }

    async fn handle_query_usage(
        &self,
        _request: QueryUsageRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<QueryUsageResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::USAGE_QUERY.to_string(),
        ))
    }

    async fn handle_query_trace(
        &self,
        _request: QueryRouteTraceRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<QueryRouteTraceResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::TRACE_QUERY.to_string(),
        ))
    }

    async fn handle_provider_catalog(
        &self,
        _request: ProviderCatalogRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderCatalogResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_CATALOG.to_string(),
        ))
    }

    async fn handle_list_protocol_adapters(
        &self,
        _request: ProtocolAdapterListRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProtocolAdapterListResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROTOCOL_ADAPTER_LIST.to_string(),
        ))
    }

    async fn handle_validate_provider(
        &self,
        _request: ProviderValidateRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderValidateResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_VALIDATE.to_string(),
        ))
    }

    async fn handle_add_provider(
        &self,
        _request: ProviderAddRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderAddResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_ADD.to_string(),
        ))
    }

    async fn handle_list_providers(
        &self,
        _request: ProviderListRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderListResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_LIST.to_string(),
        ))
    }

    async fn handle_provider_health(
        &self,
        _request: ProviderHealthRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderHealthResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_HEALTH.to_string(),
        ))
    }

    async fn handle_update_provider(
        &self,
        _request: ProviderUpdateRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderUpdateResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_UPDATE.to_string(),
        ))
    }

    async fn handle_delete_provider(
        &self,
        _request: ProviderDeleteRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderDeleteResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_DELETE.to_string(),
        ))
    }

    async fn handle_refresh_provider_models(
        &self,
        _request: ProviderRefreshModelsRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<ProviderRefreshModelsResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::PROVIDER_REFRESH_MODELS.to_string(),
        ))
    }

    async fn handle_list_models(
        &self,
        _request: ListModelsRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<Value, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::MODELS_LIST.to_string(),
        ))
    }

    async fn handle_driver_metadata_update_get(
        &self,
        _ctx: RPCContext,
    ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::DRIVER_METADATA_UPDATE_GET.to_string(),
        ))
    }

    async fn handle_driver_metadata_update_set(
        &self,
        _request: DriverMetadataUpdateSetReq,
        _ctx: RPCContext,
    ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::DRIVER_METADATA_UPDATE_SET.to_string(),
        ))
    }
}

pub struct AiccServerHandler<T: AiccHandler>(pub T);

impl<T: AiccHandler> AiccServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: AiccHandler> RPCHandler for AiccServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let method = req.method.clone();
        let result = match method.as_str() {
            ai_methods::CANCEL => {
                let cancel_req = CancelRequest::from_json(req.params)?;
                let result = self.0.handle_cancel(&cancel_req.task_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::ROUTE_RESOLVE => {
                let route_req = RouteResolveRequest::from_json(req.params)?;
                let result = self.0.handle_route_resolve(route_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::CHAT_COMPLETIONS_CREATE => {
                let invoke_req = LlmChatInvokeRequest::from_json(req.params)?;
                invoke_req
                    .messages
                    .iter()
                    .try_for_each(AiMessage::validate)
                    .map_err(|err| {
                        RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}"))
                    })?;
                let result = self
                    .0
                    .handle_chat_completions_create(invoke_req, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::IMAGES_GENERATE => {
                let invoke_req = TextToImageInvokeRequest::from_json(req.params)?;
                let result = self.0.handle_images_generate(invoke_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::HELPER_LLM_CHAT => {
                let method_req = LlmChatHelperRequest::from_json(req.params)?;
                let result = self.0.handle_helper_llm_chat(method_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::HELPER_TEXT_TO_IMAGE => {
                let method_req = TextToImageHelperRequest::from_json(req.params)?;
                let result = self.0.handle_helper_text_to_image(method_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::DRIVER_METADATA_UPDATE_GET => {
                DriverMetadataUpdateGetReq::from_json(req.params)?;
                let result = self.0.handle_driver_metadata_update_get(ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::DRIVER_METADATA_UPDATE_SET => {
                let update_req = DriverMetadataUpdateSetReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_driver_metadata_update_set(update_req, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::EMBEDDING_TEXT => RPCResult::Success(json!(
                self.0
                    .handle_embedding_text(EmbeddingTextRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::EMBEDDING_MULTIMODAL => RPCResult::Success(json!(
                self.0
                    .handle_embedding_multimodal(
                        EmbeddingMultimodalRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::RERANK => RPCResult::Success(json!(
                self.0
                    .handle_rerank(RerankRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::IMAGE_IMG2IMG => RPCResult::Success(json!(
                self.0
                    .handle_image_to_image(ImageToImageRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::IMAGE_INPAINT => RPCResult::Success(json!(
                self.0
                    .handle_image_inpaint(ImageInpaintRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::IMAGE_UPSCALE => RPCResult::Success(json!(
                self.0
                    .handle_image_upscale(ImageUpscaleRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::IMAGE_BG_REMOVE => RPCResult::Success(json!(
                self.0
                    .handle_image_background_remove(
                        ImageBackgroundRemoveRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::VISION_OCR => RPCResult::Success(json!(
                self.0
                    .handle_vision_ocr(VisionOcrRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VISION_CAPTION => RPCResult::Success(json!(
                self.0
                    .handle_vision_caption(VisionCaptionRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VISION_DETECT => RPCResult::Success(json!(
                self.0
                    .handle_vision_detect(VisionDetectRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VISION_SEGMENT => RPCResult::Success(json!(
                self.0
                    .handle_vision_segment(VisionSegmentRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::AUDIO_TTS => RPCResult::Success(json!(
                self.0
                    .handle_audio_text_to_speech(
                        AudioTextToSpeechRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::AUDIO_ASR => RPCResult::Success(json!(
                self.0
                    .handle_audio_speech_recognition(
                        AudioSpeechRecognitionRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::AUDIO_MUSIC => RPCResult::Success(json!(
                self.0
                    .handle_audio_music(AudioMusicRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::AUDIO_ENHANCE => RPCResult::Success(json!(
                self.0
                    .handle_audio_enhance(AudioEnhanceRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VIDEO_TXT2VIDEO => RPCResult::Success(json!(
                self.0
                    .handle_video_text_to_video(
                        VideoTextToVideoRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::VIDEO_IMG2VIDEO => RPCResult::Success(json!(
                self.0
                    .handle_video_image_to_video(
                        VideoImageToVideoRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::VIDEO_VIDEO2VIDEO => RPCResult::Success(json!(
                self.0
                    .handle_video_to_video(VideoToVideoRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VIDEO_EXTEND => RPCResult::Success(json!(
                self.0
                    .handle_video_extend(VideoExtendRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::VIDEO_UPSCALE => RPCResult::Success(json!(
                self.0
                    .handle_video_upscale(VideoUpscaleRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::AGENT_COMPUTER_USE => RPCResult::Success(json!(
                self.0
                    .handle_computer_use(ComputerUseRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::SERVICE_RELOAD_SETTINGS => RPCResult::Success(json!(
                self.0
                    .handle_reload_settings(
                        ServiceReloadSettingsRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::QUOTA_QUERY => RPCResult::Success(json!(
                self.0
                    .handle_query_quota(QuotaQueryRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::USAGE_QUERY => RPCResult::Success(json!(
                self.0
                    .handle_query_usage(QueryUsageRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::TRACE_QUERY => RPCResult::Success(json!(
                self.0
                    .handle_query_trace(QueryRouteTraceRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_CATALOG => RPCResult::Success(json!(
                self.0
                    .handle_provider_catalog(ProviderCatalogRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROTOCOL_ADAPTER_LIST => RPCResult::Success(json!(
                self.0
                    .handle_list_protocol_adapters(
                        ProtocolAdapterListRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::PROVIDER_VALIDATE => RPCResult::Success(json!(
                self.0
                    .handle_validate_provider(ProviderValidateRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_ADD => RPCResult::Success(json!(
                self.0
                    .handle_add_provider(ProviderAddRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_LIST => RPCResult::Success(json!(
                self.0
                    .handle_list_providers(ProviderListRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_HEALTH => RPCResult::Success(json!(
                self.0
                    .handle_provider_health(ProviderHealthRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_UPDATE => RPCResult::Success(json!(
                self.0
                    .handle_update_provider(ProviderUpdateRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_DELETE => RPCResult::Success(json!(
                self.0
                    .handle_delete_provider(ProviderDeleteRequest::from_json(req.params)?, ctx)
                    .await?
            )),
            ai_methods::PROVIDER_REFRESH_MODELS => RPCResult::Success(json!(
                self.0
                    .handle_refresh_provider_models(
                        ProviderRefreshModelsRequest::from_json(req.params)?,
                        ctx
                    )
                    .await?
            )),
            ai_methods::MODELS_LIST => RPCResult::Success(
                self.0
                    .handle_list_models(ListModelsRequest::from_json(req.params)?, ctx)
                    .await?,
            ),
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub fn generate_aicc_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        AICC_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("AI Compute Center")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}
