use crate::model_registry::ModelRegistry;
use crate::model_session::{LogicalNode, SessionConfig};
use crate::model_types::{
    ApiType, FallbackMode, FallbackRule, LogicalModelDefinition, ModelDisable, ModelItem,
    ModelRequirement, MountMode, SchedulerProfile,
};
use anyhow::{anyhow, Context, Result};
use buckyos_kit::get_buckyos_system_etc_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_LOGICAL_TREE_REVISION: &str = "builtin-aicc-router-v4";
pub const LOCAL_LOGICAL_TREE_SCHEMA_VERSION: u32 = 1;
pub const LOCAL_LOGICAL_TREE_FILE_NAME: &str = "default_logical_tree.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalLogicalTreeConfig {
    pub schema_version: u32,
    pub revision: String,
    #[serde(default)]
    pub logical_definitions: Vec<LogicalModelDefinition>,
    #[serde(default)]
    pub logical_tree: BTreeMap<String, LogicalNode>,
}

#[derive(Clone, Copy)]
enum FallbackPreset {
    Parent,
    #[allow(dead_code)]
    Strict,
    Disabled,
}

struct Level2Template {
    path: &'static str,
    fallback: FallbackPreset,
    profile: Option<SchedulerProfile>,
    min_line: ModelRequirementTemplate,
    disable_line: ModelDisableTemplate,
    mount_mode: MountMode,
    tier: &'static str,
}

#[derive(Clone, Copy)]
struct ModelRequirementTemplate {
    streaming: bool,
    tool_call: bool,
    json_schema: bool,
    web_search: bool,
    vision: bool,
    min_context_tokens: Option<u64>,
}

impl ModelRequirementTemplate {
    const fn empty() -> Self {
        Self {
            streaming: false,
            tool_call: false,
            json_schema: false,
            web_search: false,
            vision: false,
            min_context_tokens: None,
        }
    }

    const fn tool_json(min_context_tokens: u64) -> Self {
        Self {
            streaming: false,
            tool_call: true,
            json_schema: true,
            web_search: false,
            vision: false,
            min_context_tokens: Some(min_context_tokens),
        }
    }

    const fn vision(min_context_tokens: u64) -> Self {
        Self {
            streaming: false,
            tool_call: false,
            json_schema: false,
            web_search: false,
            vision: true,
            min_context_tokens: Some(min_context_tokens),
        }
    }

    fn to_model_requirement(self) -> ModelRequirement {
        ModelRequirement {
            streaming: self.streaming,
            tool_call: self.tool_call,
            json_schema: self.json_schema,
            web_search: self.web_search,
            vision: self.vision,
            min_context_tokens: self.min_context_tokens,
        }
    }
}

#[derive(Clone, Copy)]
struct ModelDisableTemplate {
    streaming: bool,
    tool_call: bool,
    json_schema: bool,
    web_search: bool,
    vision: bool,
    min_context_tokens: Option<u64>,
}

impl ModelDisableTemplate {
    const fn empty() -> Self {
        Self {
            streaming: false,
            tool_call: false,
            json_schema: false,
            web_search: false,
            vision: false,
            min_context_tokens: None,
        }
    }

    fn to_model_disable(self) -> ModelDisable {
        ModelDisable {
            streaming: self.streaming,
            tool_call: self.tool_call,
            json_schema: self.json_schema,
            web_search: self.web_search,
            vision: self.vision,
            min_context_tokens: self.min_context_tokens,
        }
    }
}

const LLM_TEMPLATES: &[Level2Template] = &[
    Level2Template {
        path: "llm.chat",
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::Balanced),
        min_line: ModelRequirementTemplate::empty(),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "general",
    },
    Level2Template {
        path: "llm.plan",
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::QualityFirst),
        min_line: ModelRequirementTemplate::tool_json(32_768),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "pro",
    },
    Level2Template {
        path: "llm.code",
        fallback: FallbackPreset::Parent,
        profile: None,
        min_line: ModelRequirementTemplate::tool_json(32_768),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "pro",
    },
    Level2Template {
        path: "llm.swift",
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::LatencyFirst),
        min_line: ModelRequirementTemplate::empty(),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "fast",
    },
    Level2Template {
        path: "llm.summarize",
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::CostFirst),
        min_line: ModelRequirementTemplate {
            min_context_tokens: Some(16_384),
            ..ModelRequirementTemplate::empty()
        },
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "utility",
    },
    Level2Template {
        path: "llm.reason",
        fallback: FallbackPreset::Disabled,
        profile: Some(SchedulerProfile::QualityFirst),
        min_line: ModelRequirementTemplate {
            min_context_tokens: Some(32_768),
            ..ModelRequirementTemplate::empty()
        },
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "reasoning",
    },
    Level2Template {
        path: "llm.vision",
        fallback: FallbackPreset::Parent,
        profile: None,
        min_line: ModelRequirementTemplate::vision(32_768),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "multimodal",
    },
    Level2Template {
        path: "llm.long",
        fallback: FallbackPreset::Parent,
        profile: None,
        min_line: ModelRequirementTemplate {
            min_context_tokens: Some(128_000),
            ..ModelRequirementTemplate::empty()
        },
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "long_context",
    },
    Level2Template {
        path: "llm.fallback",
        fallback: FallbackPreset::Disabled,
        profile: None,
        min_line: ModelRequirementTemplate::empty(),
        disable_line: ModelDisableTemplate::empty(),
        mount_mode: MountMode::Hybrid,
        tier: "fallback",
    },
];

fn fallback_to_rule(preset: &FallbackPreset) -> FallbackRule {
    match preset {
        FallbackPreset::Parent => FallbackRule::parent(),
        FallbackPreset::Strict => FallbackRule::strict(),
        FallbackPreset::Disabled => FallbackRule {
            mode: FallbackMode::Disabled,
            target: None,
        },
    }
}

fn logical_definition(
    path: &str,
    api_type: ApiType,
    fallback: FallbackPreset,
    profile: Option<SchedulerProfile>,
    min_line: ModelRequirement,
    mount_mode: MountMode,
    tier: &str,
) -> LogicalModelDefinition {
    LogicalModelDefinition {
        path: path.to_string(),
        api_type,
        min_line,
        disable_line: ModelDisable::default(),
        default_options: None,
        mount_mode,
        scheduler_profile: profile,
        fallback: Some(fallback_to_rule(&fallback)),
        route_policy: None,
        user_visible_tier: Some(tier.to_string()),
    }
}

pub fn build_default_logical_definitions() -> Vec<LogicalModelDefinition> {
    let mut definitions = vec![logical_definition(
        "llm",
        ApiType::Llm,
        FallbackPreset::Parent,
        Some(SchedulerProfile::Balanced),
        ModelRequirement::default(),
        MountMode::Auto,
        "general",
    )];

    for template in LLM_TEMPLATES {
        let mut definition = logical_definition(
            template.path,
            ApiType::Llm,
            template.fallback,
            template.profile.clone(),
            template.min_line.to_model_requirement(),
            template.mount_mode.clone(),
            template.tier,
        );
        definition.disable_line = template.disable_line.to_model_disable();
        definitions.push(definition);
    }

    definitions.extend([
        logical_definition(
            "llm.summary",
            ApiType::Llm,
            FallbackPreset::Parent,
            Some(SchedulerProfile::CostFirst),
            ModelRequirementTemplate {
                min_context_tokens: Some(16_384),
                ..ModelRequirementTemplate::empty()
            }
            .to_model_requirement(),
            MountMode::Hybrid,
            "utility",
        ),
        logical_definition(
            "llm.translate",
            ApiType::Llm,
            FallbackPreset::Parent,
            Some(SchedulerProfile::CostFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "utility",
        ),
        logical_definition(
            "embedding.text",
            ApiType::Embedding,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "embedding.multilingual",
            ApiType::Embedding,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "embedding.code",
            ApiType::Embedding,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "code",
        ),
        logical_definition(
            "embedding.multimodal",
            ApiType::EmbeddingMultimodal,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "multimodal",
        ),
        logical_definition(
            "rerank.general",
            ApiType::Rerank,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "rerank.multilingual",
            ApiType::Rerank,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "image.txt2img",
            ApiType::ImageTextToImage,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "image.img2img",
            ApiType::ImageToImage,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "general",
        ),
        logical_definition(
            "image.inpaint",
            ApiType::ImageInpaint,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "edit",
        ),
        logical_definition(
            "image.upscale",
            ApiType::ImageUpscale,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "edit",
        ),
        logical_definition(
            "image.bg_remove",
            ApiType::ImageBgRemove,
            FallbackPreset::Parent,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "utility",
        ),
        logical_definition(
            "image.ocr",
            ApiType::VisionOcr,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "vision",
        ),
        logical_definition(
            "image.caption",
            ApiType::VisionCaption,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "vision",
        ),
        logical_definition(
            "image.detect",
            ApiType::VisionDetect,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "vision",
        ),
        logical_definition(
            "image.segment",
            ApiType::VisionSegment,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "vision",
        ),
        logical_definition(
            "audio.tts",
            ApiType::AudioTts,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "audio",
        ),
        logical_definition(
            "audio.asr",
            ApiType::AudioAsr,
            FallbackPreset::Strict,
            Some(SchedulerProfile::LatencyFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "audio",
        ),
        logical_definition(
            "audio.music",
            ApiType::AudioMusic,
            FallbackPreset::Strict,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "audio",
        ),
        logical_definition(
            "audio.enhance",
            ApiType::AudioEnhance,
            FallbackPreset::Strict,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "audio",
        ),
        logical_definition(
            "video.txt2video",
            ApiType::VideoTextToVideo,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "video",
        ),
        logical_definition(
            "video.img2video",
            ApiType::VideoImageToVideo,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "video",
        ),
        logical_definition(
            "video.video2video",
            ApiType::VideoToVideo,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "video",
        ),
        logical_definition(
            "video.extend",
            ApiType::VideoExtend,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "video",
        ),
        logical_definition(
            "video.upscale",
            ApiType::VideoUpscale,
            FallbackPreset::Parent,
            Some(SchedulerProfile::QualityFirst),
            ModelRequirement::default(),
            MountMode::Hybrid,
            "video",
        ),
        logical_definition(
            "agent.computer_use",
            ApiType::AgentComputerUse,
            FallbackPreset::Parent,
            Some(SchedulerProfile::Balanced),
            ModelRequirementTemplate::vision(8_192).to_model_requirement(),
            MountMode::Hybrid,
            "agent",
        ),
    ]);

    definitions.sort_by(|left, right| left.path.cmp(&right.path));
    if let Some(index) = definitions
        .iter()
        .position(|definition| definition.path == "llm")
    {
        let llm = definitions.remove(index);
        definitions.insert(0, llm);
    }

    definitions
}

fn model_item(target: &str, weight: f64) -> ModelItem {
    ModelItem::new(target.to_string(), weight)
}

fn logical_node_with_items(items: &[(&str, &str, f64)]) -> LogicalNode {
    LogicalNode {
        source: Some(DEFAULT_LOGICAL_TREE_REVISION.to_string()),
        items: Some(
            items
                .iter()
                .map(|(name, target, weight)| ((*name).to_string(), model_item(target, *weight)))
                .collect(),
        ),
        ..Default::default()
    }
}

fn empty_logical_node() -> LogicalNode {
    LogicalNode {
        source: Some(DEFAULT_LOGICAL_TREE_REVISION.to_string()),
        ..Default::default()
    }
}

pub fn build_default_logical_tree() -> BTreeMap<String, LogicalNode> {
    let llm_children = [
        (
            "chat".to_string(),
            logical_node_with_items(&[
                ("gpt", "llm.gpt-standard", 2.2),
                ("sonnet", "llm.sonnet", 2.1),
                ("gemini", "llm.gemini-flash", 1.9),
                ("mini", "llm.gpt-mini", 1.4),
            ]),
        ),
        (
            "plan".to_string(),
            logical_node_with_items(&[
                ("opus", "llm.opus", 2.5),
                ("gemini", "llm.gemini-pro", 2.4),
                ("gpt_pro", "llm.gpt-pro", 2.3),
                ("gpt", "llm.gpt-standard", 2.0),
                ("local_code", "llm.qwen-coder", 1.1),
            ]),
        ),
        (
            "code".to_string(),
            logical_node_with_items(&[
                ("sonnet", "llm.sonnet", 2.4),
                ("gpt", "llm.gpt-standard", 2.1),
                ("local", "llm.qwen-coder", 1.8),
                ("opus", "llm.opus", 1.4),
            ]),
        ),
        (
            "swift".to_string(),
            logical_node_with_items(&[
                ("mini", "llm.gpt-mini", 2.0),
                ("haiku", "llm.haiku", 1.9),
                ("gemini_flash", "llm.gemini-flash", 1.8),
                ("gemini_flash_lite", "llm.gemini-flash-lite", 1.6),
                ("local_small", "llm.qwen-small", 1.4),
            ]),
        ),
        (
            "summarize".to_string(),
            logical_node_with_items(&[
                ("mini", "llm.gpt-mini", 2.0),
                ("gemini_flash", "llm.gemini-flash", 1.8),
                ("haiku", "llm.haiku", 1.6),
                ("local_small", "llm.qwen-small", 1.3),
            ]),
        ),
        (
            "summary".to_string(),
            logical_node_with_items(&[
                ("mini", "llm.gpt-mini", 2.0),
                ("gemini_flash", "llm.gemini-flash", 1.8),
                ("haiku", "llm.haiku", 1.6),
                ("local_small", "llm.qwen-small", 1.3),
            ]),
        ),
        (
            "translate".to_string(),
            logical_node_with_items(&[
                ("gemini_flash", "llm.gemini-flash", 1.9),
                ("mini", "llm.gpt-mini", 1.8),
                ("haiku", "llm.haiku", 1.6),
                ("local_small", "llm.qwen-small", 1.2),
            ]),
        ),
        (
            "reason".to_string(),
            logical_node_with_items(&[
                ("gpt_pro", "llm.gpt-pro", 2.5),
                ("opus", "llm.opus", 2.4),
                ("gemini", "llm.gemini-pro", 2.2),
            ]),
        ),
        (
            "vision".to_string(),
            logical_node_with_items(&[
                ("gpt", "llm.gpt-standard", 2.2),
                ("gemini", "llm.gemini-pro", 2.1),
                ("opus", "llm.opus", 1.9),
                ("sonnet", "llm.sonnet", 1.8),
            ]),
        ),
        (
            "long".to_string(),
            logical_node_with_items(&[
                ("gemini", "llm.gemini-pro", 2.3),
                ("opus", "llm.opus", 2.1),
                ("gpt_pro", "llm.gpt-pro", 2.0),
                ("gpt", "llm.gpt-standard", 1.8),
            ]),
        ),
        ("fallback".to_string(), empty_logical_node()),
        ("gpt-standard".to_string(), empty_logical_node()),
        ("gpt-pro".to_string(), empty_logical_node()),
        ("gpt-mini".to_string(), empty_logical_node()),
        ("opus".to_string(), empty_logical_node()),
        ("sonnet".to_string(), empty_logical_node()),
        ("haiku".to_string(), empty_logical_node()),
        ("gemini-pro".to_string(), empty_logical_node()),
        ("gemini-flash".to_string(), empty_logical_node()),
        ("gemini-flash-lite".to_string(), empty_logical_node()),
        ("qwen-coder".to_string(), empty_logical_node()),
        ("qwen-small".to_string(), empty_logical_node()),
    ]
    .into_iter()
    .collect();

    [(
        "llm".to_string(),
        LogicalNode {
            source: Some(DEFAULT_LOGICAL_TREE_REVISION.to_string()),
            children: llm_children,
            ..Default::default()
        },
    )]
    .into_iter()
    .collect()
}

pub fn local_logical_tree_config_path() -> PathBuf {
    get_buckyos_system_etc_dir()
        .join("aicc")
        .join(LOCAL_LOGICAL_TREE_FILE_NAME)
}

pub fn build_builtin_local_logical_tree_config() -> LocalLogicalTreeConfig {
    LocalLogicalTreeConfig {
        schema_version: LOCAL_LOGICAL_TREE_SCHEMA_VERSION,
        revision: DEFAULT_LOGICAL_TREE_REVISION.to_string(),
        logical_definitions: build_default_logical_definitions(),
        logical_tree: build_default_logical_tree(),
    }
}

pub fn load_or_create_local_logical_tree_config() -> Result<LocalLogicalTreeConfig> {
    let path = local_logical_tree_config_path();
    match std::fs::read_to_string(path.as_path()) {
        Ok(content) => {
            if let Ok(config) = parse_local_logical_tree_config(content.as_str()) {
                if config.revision == DEFAULT_LOGICAL_TREE_REVISION {
                    return Ok(config);
                }
            }
            let config = build_builtin_local_logical_tree_config();
            write_local_logical_tree_config(&path, &config)?;
            Ok(config)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let config = build_builtin_local_logical_tree_config();
            write_local_logical_tree_config(&path, &config)?;
            Ok(config)
        }
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn write_local_logical_tree_config(path: &PathBuf, config: &LocalLogicalTreeConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(config)
        .context("serialize builtin local logical tree config")?;
    std::fs::write(path.as_path(), content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn parse_local_logical_tree_config(content: &str) -> Result<LocalLogicalTreeConfig> {
    let config: LocalLogicalTreeConfig = serde_json::from_str(content)?;
    if config.schema_version != LOCAL_LOGICAL_TREE_SCHEMA_VERSION {
        return Err(anyhow!(
            "unsupported schema_version {}, expected {}",
            config.schema_version,
            LOCAL_LOGICAL_TREE_SCHEMA_VERSION
        ));
    }
    if config.logical_definitions.is_empty() {
        return Err(anyhow!("logical_definitions must not be empty"));
    }
    let mut registry = ModelRegistry::new();
    registry.set_logical_definitions(config.logical_definitions.clone())?;
    SessionConfig {
        logical_tree: config.logical_tree.clone(),
        ..Default::default()
    }
    .validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_local_logical_tree_config_round_trips() {
        let config = build_builtin_local_logical_tree_config();
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded = parse_local_logical_tree_config(encoded.as_str()).unwrap();

        assert_eq!(decoded.schema_version, LOCAL_LOGICAL_TREE_SCHEMA_VERSION);
        assert_eq!(decoded.revision, DEFAULT_LOGICAL_TREE_REVISION);
        assert!(decoded
            .logical_definitions
            .iter()
            .any(|definition| definition.path == "llm.plan"));
        assert!(serde_json::to_value(&decoded)
            .unwrap()
            .get("session_config")
            .is_none());
        assert_eq!(
            decoded.logical_tree["llm"].children["plan"]
                .items
                .as_ref()
                .unwrap()["opus"]
                .target,
            "llm.opus"
        );
        assert!(
            decoded
                .logical_definitions
                .iter()
                .any(|definition| definition.path == "llm.chat"
                    && definition.api_type == ApiType::Llm)
        );
    }

    #[test]
    fn local_logical_tree_config_requires_definitions() {
        let mut value = serde_json::to_value(build_builtin_local_logical_tree_config()).unwrap();
        value["logical_definitions"] = serde_json::json!([]);

        let err = parse_local_logical_tree_config(value.to_string().as_str()).unwrap_err();
        assert!(err
            .to_string()
            .contains("logical_definitions must not be empty"));
    }

    #[test]
    fn local_logical_tree_config_rejects_session_config() {
        let mut value = serde_json::to_value(build_builtin_local_logical_tree_config()).unwrap();
        value["session_config"] = serde_json::json!({});

        let err = parse_local_logical_tree_config(value.to_string().as_str()).unwrap_err();
        assert!(err.to_string().contains("unknown field `session_config`"));
    }

    #[test]
    fn builtin_logical_definitions_have_expected_paths() {
        let definitions = build_default_logical_definitions();
        let paths = definitions
            .iter()
            .map(|definition| definition.path.as_str())
            .collect::<Vec<_>>();
        for path in [
            "llm",
            "llm.chat",
            "llm.plan",
            "llm.code",
            "llm.swift",
            "llm.summarize",
            "llm.reason",
            "llm.vision",
            "llm.long",
            "llm.fallback",
            "llm.summary",
            "llm.translate",
            "embedding.text",
            "embedding.multilingual",
            "embedding.code",
            "embedding.multimodal",
            "rerank.general",
            "rerank.multilingual",
            "image.txt2img",
            "image.img2img",
            "image.inpaint",
            "image.upscale",
            "image.bg_remove",
            "image.ocr",
            "image.caption",
            "image.detect",
            "image.segment",
            "audio.tts",
            "audio.asr",
            "audio.music",
            "audio.enhance",
            "video.txt2video",
            "video.img2video",
            "video.video2video",
            "video.extend",
            "video.upscale",
            "agent.computer_use",
        ] {
            assert!(paths.contains(&path), "{} should be present", path);
        }
    }

    #[test]
    fn builtin_logical_definitions_keep_fallback_modes() {
        let definitions = build_default_logical_definitions();
        for path in ["llm.reason", "llm.fallback"] {
            let definition = definitions
                .iter()
                .find(|definition| definition.path == path)
                .unwrap();
            assert_eq!(
                definition.fallback.as_ref().map(|rule| rule.mode.clone()),
                Some(FallbackMode::Disabled),
                "{} should have fallback mode disabled",
                path
            );
        }
    }

    #[test]
    fn builtin_logical_tree_has_weighted_family_routes() {
        let tree = build_default_logical_tree();
        let plan = tree["llm"].children["plan"].items.as_ref().unwrap();
        assert_eq!(plan["opus"].target, "llm.opus");
        assert_eq!(plan["gemini"].target, "llm.gemini-pro");
        assert_eq!(plan["gpt_pro"].target, "llm.gpt-pro");
        assert!(plan["opus"].weight > plan["local_code"].weight);

        let code = tree["llm"].children["code"].items.as_ref().unwrap();
        assert_eq!(code["sonnet"].target, "llm.sonnet");
        assert_eq!(code["local"].target, "llm.qwen-coder");
    }
}
