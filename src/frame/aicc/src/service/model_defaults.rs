use async_trait::async_trait;
use buckyos_api::{
    AiccFallbackMode, AiccFallbackRule, AiccLogicalNodeOverlay, AiccRouteOverlay,
    AiccSchedulerProfile, ApiType, ModelDisable, ModelItem, ModelRequirement,
};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::catalog::CatalogSnapshot;
use crate::error::RuntimeError;
use crate::model::{
    LogicalModelDefinition, ModelRegistry, MountMode, ProviderInventory as ModelProviderInventory,
    RegistryLayers,
};
use crate::runtime::ModelRegistryAssembler;

pub(super) struct ServiceModelAssembler {
    pub(super) session: Option<buckyos_api::AiccRouteOverlay>,
}

#[async_trait]
impl ModelRegistryAssembler for ServiceModelAssembler {
    async fn build(
        &self,
        catalog: Arc<CatalogSnapshot>,
        inventories: Vec<ModelProviderInventory>,
    ) -> Result<Arc<ModelRegistry>, RuntimeError> {
        ModelRegistry::build(
            catalog.as_ref(),
            &inventories,
            builtin_logical_model_definitions(),
            RegistryLayers {
                factory: Some(&builtin_logical_tree_overlay()),
                session: self.session.as_ref(),
                ..RegistryLayers::default()
            },
        )
        .map(Arc::new)
        .map_err(|error| RuntimeError::Backend(error.to_string()))
    }
}

pub(super) fn builtin_logical_model_definitions() -> Vec<LogicalModelDefinition> {
    let mut definitions = vec![
        llm_logical_definition(
            "llm",
            ModelRequirement::default(),
            MountMode::Auto,
            AiccSchedulerProfile::Balanced,
            Some(AiccFallbackRule {
                mode: AiccFallbackMode::Strict,
                target: None,
            }),
            Some("general"),
        ),
        logical_definition(
            "embedding.text",
            ApiType::EmbeddingText,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("general"),
        ),
        logical_definition(
            "embedding.multimodal",
            ApiType::EmbeddingMultimodal,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("multimodal"),
        ),
        logical_definition(
            "rerank",
            ApiType::Rerank,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("general"),
        ),
    ];
    definitions.extend([
        llm_logical_definition(
            "llm.chat",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("general"),
        ),
        llm_logical_definition(
            "llm.plan",
            tool_json_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("pro"),
        ),
        llm_logical_definition(
            "llm.code",
            tool_json_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("pro"),
        ),
        llm_logical_definition(
            "llm.swift",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            parent_fallback(),
            Some("fast"),
        ),
        llm_logical_definition(
            "llm.summarize",
            context_requirement(16_384),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.summary",
            context_requirement(16_384),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.translate",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.reason",
            context_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            disabled_fallback(),
            Some("reasoning"),
        ),
        llm_logical_definition(
            "llm.vision",
            vision_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("multimodal"),
        ),
        llm_logical_definition(
            "llm.long",
            context_requirement(128_000),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("long_context"),
        ),
        llm_logical_definition(
            "llm.fallback",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            disabled_fallback(),
            Some("fallback"),
        ),
    ]);
    definitions.extend([
        logical_definition(
            "image.txt2img",
            ApiType::ImageTextToImage,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("general"),
        ),
        logical_definition(
            "image.img2img",
            ApiType::ImageImageToImage,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("general"),
        ),
        logical_definition(
            "image.inpaint",
            ApiType::ImageInpaint,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("edit"),
        ),
        logical_definition(
            "image.upscale",
            ApiType::ImageUpscale,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("edit"),
        ),
        logical_definition(
            "image.bg_remove",
            ApiType::ImageBackgroundRemove,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            parent_fallback(),
            Some("utility"),
        ),
        logical_definition(
            "vision.ocr",
            ApiType::VisionOcr,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.caption",
            ApiType::VisionCaption,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.detect",
            ApiType::VisionDetect,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.segment",
            ApiType::VisionSegment,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "audio.tts",
            ApiType::AudioTextToSpeech,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.asr",
            ApiType::AudioSpeechRecognition,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.music",
            ApiType::AudioMusic,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.enhance",
            ApiType::AudioEnhance,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "video.txt2video",
            ApiType::VideoTextToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.img2video",
            ApiType::VideoImageToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.video2video",
            ApiType::VideoToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.extend",
            ApiType::VideoExtend,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.upscale",
            ApiType::VideoUpscale,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "agent_runtime.computer_use",
            ApiType::AgentComputerUse,
            vision_requirement(8_192),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("agent"),
        ),
    ]);
    definitions
}

fn llm_logical_definition(
    path: &str,
    min_line: ModelRequirement,
    mount_mode: MountMode,
    scheduler_profile: AiccSchedulerProfile,
    fallback: Option<AiccFallbackRule>,
    tier: Option<&str>,
) -> LogicalModelDefinition {
    logical_definition(
        path,
        ApiType::Llm,
        min_line,
        mount_mode,
        scheduler_profile,
        fallback,
        tier,
    )
}

fn logical_definition(
    path: &str,
    api_type: ApiType,
    min_line: ModelRequirement,
    mount_mode: MountMode,
    scheduler_profile: AiccSchedulerProfile,
    fallback: Option<AiccFallbackRule>,
    tier: Option<&str>,
) -> LogicalModelDefinition {
    LogicalModelDefinition {
        path: path.to_string(),
        api_type,
        min_line,
        disable_line: ModelDisable::default(),
        default_options: BTreeMap::new(),
        mount_mode,
        scheduler_profile,
        fallback,
        route_policy: buckyos_api::AiccPolicyConfig::default(),
        user_visible_tier: tier.map(str::to_owned),
    }
}

fn parent_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Parent,
        target: None,
    })
}

fn strict_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Strict,
        target: None,
    })
}

fn disabled_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Disabled,
        target: None,
    })
}

fn tool_json_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        tool_call: true,
        json_schema: true,
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

fn context_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

fn vision_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        vision: true,
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

pub(super) fn builtin_logical_tree_overlay() -> AiccRouteOverlay {
    AiccRouteOverlay {
        revision: Some("builtin-aicc-router-v4".to_string()),
        logical_tree: BTreeMap::from([(
            "llm".to_string(),
            AiccLogicalNodeOverlay {
                children: BTreeMap::from([
                    (
                        "chat".to_string(),
                        logical_node(&[
                            ("gpt", "llm.gpt-standard", 2.2),
                            ("sonnet", "llm.sonnet", 2.1),
                            ("gemini", "llm.gemini-flash", 1.9),
                            ("mini", "llm.gpt-mini", 1.4),
                            ("qwen_plus", "llm.qwen-plus", 1.3),
                            ("glm", "llm.glm", 1.2),
                            ("kimi", "llm.kimi", 1.2),
                            ("deepseek_flash", "llm.deepseek-flash", 1.1),
                            ("doubao_lite", "llm.doubao-lite", 1.0),
                            ("minimax", "llm.minimax", 0.9),
                        ]),
                    ),
                    (
                        "plan".to_string(),
                        logical_node(&[
                            ("opus", "llm.opus", 2.5),
                            ("gemini", "llm.gemini-pro", 2.4),
                            ("gpt_pro", "llm.gpt-pro", 2.3),
                            ("gpt", "llm.gpt-standard", 2.0),
                            ("qwen_max", "llm.qwen-max", 1.8),
                            ("deepseek", "llm.deepseek-pro", 1.5),
                            ("doubao_pro", "llm.doubao-pro", 1.4),
                            ("glm", "llm.glm", 1.3),
                            ("kimi", "llm.kimi", 1.2),
                            ("minimax", "llm.minimax", 1.1),
                        ]),
                    ),
                    (
                        "code".to_string(),
                        logical_node(&[
                            ("sonnet", "llm.sonnet", 2.4),
                            ("gpt", "llm.gpt-standard", 2.1),
                            ("qwen", "llm.qwen-max", 1.9),
                            ("deepseek", "llm.deepseek-pro", 1.8),
                            ("kimi_code", "llm.kimi-code", 1.7),
                            ("doubao_code", "llm.doubao-code", 1.6),
                            ("opus", "llm.opus", 1.4),
                        ]),
                    ),
                    (
                        "swift".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("haiku", "llm.haiku", 1.9),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("gemini_flash_lite", "llm.gemini-flash-lite", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_mini", "llm.doubao-mini", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "summarize".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "summary".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "translate".to_string(),
                        logical_node(&[
                            ("gemini_flash", "llm.gemini-flash", 1.9),
                            ("mini", "llm.gpt-mini", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "reason".to_string(),
                        logical_node(&[
                            ("gpt_pro", "llm.gpt-pro", 2.5),
                            ("opus", "llm.opus", 2.4),
                            ("gemini", "llm.gemini-pro", 2.2),
                            ("qwen_max", "llm.qwen-max", 1.9),
                            ("deepseek", "llm.deepseek-pro", 1.8),
                            ("doubao_pro", "llm.doubao-pro", 1.6),
                            ("glm", "llm.glm", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                            ("minimax", "llm.minimax", 1.3),
                        ]),
                    ),
                    (
                        "vision".to_string(),
                        logical_node(&[
                            ("gpt", "llm.gpt-standard", 2.2),
                            ("gemini", "llm.gemini-pro", 2.1),
                            ("opus", "llm.opus", 1.9),
                            ("sonnet", "llm.sonnet", 1.8),
                            ("qwen_max", "llm.qwen-max", 1.6),
                            ("doubao_pro", "llm.doubao-pro", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                        ]),
                    ),
                    (
                        "long".to_string(),
                        logical_node(&[
                            ("gemini", "llm.gemini-pro", 2.3),
                            ("opus", "llm.opus", 2.1),
                            ("gpt_pro", "llm.gpt-pro", 2.0),
                            ("gpt", "llm.gpt-standard", 1.8),
                            ("qwen_max", "llm.qwen-max", 1.6),
                            ("deepseek", "llm.deepseek-pro", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                            ("minimax", "llm.minimax", 1.3),
                        ]),
                    ),
                ]),
                ..AiccLogicalNodeOverlay::default()
            },
        )]),
        ..AiccRouteOverlay::default()
    }
}

fn logical_node(items: &[(&str, &str, f64)]) -> AiccLogicalNodeOverlay {
    AiccLogicalNodeOverlay {
        items: Some(
            items
                .iter()
                .map(|(name, target, weight)| {
                    ((*name).to_string(), ModelItem::new(*target, *weight))
                })
                .collect(),
        ),
        ..AiccLogicalNodeOverlay::default()
    }
}
